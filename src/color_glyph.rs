// SPDX-License-Identifier: MIT OR Apache-2.0
//! COLRv0/COLRv1 + CPAL color glyph rasterization.
//!
//! This module is enabled by the `colr` feature. It parses the COLR paint
//! graph (version 0 and version 1) and the CPAL palette through `skrifa`,
//! rasterizes coverage masks with `zeno`, evaluates solid/gradient brushes and
//! composites the resulting layers into a premultiplied RGBA bitmap.
//!
//! All failure modes (missing tables, cycles, excessive graph depth, broken
//! gradients or transforms, oversized glyphs) produce [`ColorRasterError`] so
//! callers can fall back to the existing monochrome or replacement-glyph
//! paths instead of panicking.
//!
//! The rasterized bitmaps are cached in [`ColorGlyphCache`], keyed by font,
//! glyph, size, subpixel position, palette and render flags, with a bounded
//! memory footprint.

#[cfg(not(feature = "std"))]
use alloc::format;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

// In no_std builds the inherent float methods are unavailable; core_maths
// supplies them through the `CoreFloat` extension trait backed by libm.
#[cfg(not(feature = "std"))]
use core_maths::CoreFloat;

use skrifa::color::ColorGlyphFormat;
use skrifa::raw::tables::cpal::ColorRecord;
use skrifa::raw::TableProvider;
use skrifa::{FontRef, GlyphId, MetadataProvider};

use zeno::PathBuilder;

use crate::{CacheKey, CacheKeyFlags, Color, FontSystem};

mod blend;
mod cache;
mod gradient;
mod painter;

pub use cache::{ColorCacheKey, ColorGlyphCache, ColorGlyphImage, ColorRasterError};

use painter::RasterizingPainter;

/// Index returned by CPAL for "use the current foreground text color".
const FOREGROUND_PALETTE_INDEX: u16 = 0xFFFF;

/// Hard cap on the edge length of a rasterized color glyph, in pixels. Glyphs
/// whose transformed bounds exceed this are rejected so a malformed font
/// cannot trigger unbounded allocations.
pub const MAX_GLYPH_EDGE: u32 = 1024;

/// Maximum number of simultaneously active clip/layer states. The COLR spec
/// already limits graph depth (and skrifa reports cycles and depth errors),
/// but this guards the local state stacks as well.
const MAX_STATE_DEPTH: usize = 64;

/// Foreground color used by a brush when the palette index is `0xFFFF`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rgba8 {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

impl Rgba8 {
    fn from_color(color: Color) -> Self {
        let [r, g, b, a] = color.as_rgba();
        Self { r, g, b, a }
    }

    fn from_record(record: &ColorRecord) -> Self {
        Self {
            r: record.red,
            g: record.green,
            b: record.blue,
            a: record.alpha,
        }
    }
}

/// Parameters that select which CPAL palette is painted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PaletteSelector {
    /// Explicit palette index. `None` selects palette 0 (the font default).
    pub index: Option<u16>,
}

impl PaletteSelector {
    /// Use the font's default palette (index 0).
    pub fn default_palette() -> Self {
        Self { index: None }
    }

    /// Use a specific palette index from the CPAL table.
    pub fn indexed(index: u16) -> Self {
        Self { index: Some(index) }
    }

    fn resolved_index(self) -> u16 {
        self.index.unwrap_or(0)
    }
}

/// Resolve a CPAL palette index to an sRGB color, applying the foreground
/// fallback and missing-table/missing-entry rules.
fn resolve_palette_color(palette: Option<&[ColorRecord]>, index: u16, foreground: Rgba8) -> Rgba8 {
    if index == FOREGROUND_PALETTE_INDEX {
        return foreground;
    }
    match palette {
        Some(records) => records
            .get(usize::from(index))
            .map(Rgba8::from_record)
            // A palette index past the end of the table is malformed; treat the
            // paint as transparent rather than aborting the whole glyph.
            .unwrap_or(Rgba8 {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
            }),
        // No CPAL table: every palette-indexed paint falls back to the
        // foreground color (spec recommended behavior).
        None => foreground,
    }
}

/// Entry point shared by the uncached rasterizer and the cache.
pub(crate) fn rasterize(
    font_system: &mut FontSystem,
    cache_key: CacheKey,
    palette: PaletteSelector,
    foreground: Color,
) -> Result<ColorGlyphImage, ColorRasterError> {
    let font = font_system
        .get_font(cache_key.font_id, cache_key.font_weight)
        .ok_or(ColorRasterError::FontNotFound)?;

    let font_ref =
        FontRef::from_index(font.data(), 0).map_err(|_| ColorRasterError::FontNotFound)?;

    let glyph_id = GlyphId::new(u32::from(cache_key.glyph_id));

    let color_glyphs = font_ref.color_glyphs();
    // Prefer COLRv1, then COLRv0. If neither exists the caller should keep
    // using the monochrome outline path.
    let color_glyph = color_glyphs
        .get_with_format(glyph_id, ColorGlyphFormat::ColrV1)
        .or_else(|| color_glyphs.get_with_format(glyph_id, ColorGlyphFormat::ColrV0))
        .ok_or(ColorRasterError::NotAColorGlyph)?;

    let palette_index = palette.resolved_index();
    let mut palette_records = font_ref
        .color_palettes()
        .get(palette_index)
        .map(|p| p.colors().to_vec());
    if palette_index != 0 && palette_records.is_none() {
        // Requested palette does not exist: fall back to palette 0 instead of
        // failing, matching typical platform behavior.
        palette_records = font_ref
            .color_palettes()
            .get(0)
            .map(|p| p.colors().to_vec());
    }
    let palette_slice = palette_records.as_deref();
    let foreground = Rgba8::from_color(foreground);

    let font_size = f32::from_bits(cache_key.font_size_bits);
    if !font_size.is_finite() || font_size <= 0.0 {
        return Err(ColorRasterError::InvalidTransform);
    }

    let upem = font_ref
        .head()
        .map(|h| u32::from(h.units_per_em()))
        .unwrap_or(0);
    if upem == 0 {
        return Err(ColorRasterError::InvalidTransform);
    }

    // Variation location for the requested weight, matching the monochrome
    // scaler's wght handling.
    let location = font_ref.axes().location([(
        skrifa::Tag::new(b"wght"),
        f32::from(cache_key.font_weight.0),
    )]);

    let fake_italic = cache_key.flags.contains(CacheKeyFlags::FAKE_ITALIC);

    let mut painter = RasterizingPainter::new(
        &font_ref,
        &color_glyph,
        glyph_id,
        palette_slice,
        foreground,
        font_size,
        upem,
        cache_key.x_bin.as_float(),
        cache_key.y_bin.as_float(),
        fake_italic,
        &location,
    )?;
    color_glyph
        .paint(&location, &mut painter)
        .map_err(|paint_error| match paint_error {
            skrifa::color::PaintError::PaintCycleDetected => ColorRasterError::CycleDetected,
            skrifa::color::PaintError::DepthLimitExceeded => ColorRasterError::DepthLimitExceeded,
            other => ColorRasterError::MalformedPaint(format!("{other}")),
        })?;
    // A soft error recorded inside the painter (e.g. a broken sub-paint) is
    // surfaced here so callers fall back rather than emitting a partial
    // bitmap.
    painter.into_image()
}

/// Collects a glyph outline (in font units) into a zeno command vector.
struct ZenoPen {
    commands: Vec<zeno::Command>,
}

impl ZenoPen {
    fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }
}

impl skrifa::raw::model::pen::OutlinePen for ZenoPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.commands.move_to((x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.commands.line_to((x, y));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.commands.quad_to((cx0, cy0), (x, y));
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.commands.curve_to((cx0, cy0), (cx1, cy1), (x, y));
    }

    fn close(&mut self) {
        self.commands.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_resolution_rules() {
        let fg = Rgba8 {
            r: 1,
            g: 2,
            b: 3,
            a: 4,
        };
        // Foreground index always returns the foreground color, even with no
        // palette present.
        assert_eq!(
            resolve_palette_color(None, FOREGROUND_PALETTE_INDEX, fg),
            fg
        );

        // Missing CPAL falls back to foreground.
        assert_eq!(resolve_palette_color(None, 7, fg), fg);

        let records = [
            ColorRecord {
                blue: 10,
                green: 20,
                red: 30,
                alpha: 40,
            },
            ColorRecord {
                blue: 50,
                green: 60,
                red: 70,
                alpha: 80,
            },
        ];
        assert_eq!(
            resolve_palette_color(Some(&records), 0, fg),
            Rgba8 {
                r: 30,
                g: 20,
                b: 10,
                a: 40,
            }
        );
        // Out-of-range index is treated as transparent, not a panic.
        assert_eq!(
            resolve_palette_color(Some(&records), 42, fg),
            Rgba8 {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
            }
        );
    }

    #[test]
    fn palette_selector_defaults_to_zero() {
        assert_eq!(PaletteSelector::default().resolved_index(), 0);
        assert_eq!(PaletteSelector::default_palette().resolved_index(), 0);
        assert_eq!(PaletteSelector::indexed(3).resolved_index(), 3);
    }
}
