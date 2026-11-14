// SPDX-License-Identifier: MIT OR Apache-2.0
//! [`ColorPainter`] implementation that rasterizes the COLR paint graph into
//! a premultiplied RGBA bitmap.
//!
//! Coordinate systems:
//! * skrifa invokes every callback in *font units* (y-up).
//! * [`Affine`] on the transform stack stays in font units.
//! * The root transform maps font units to device pixels (y-down) and shifts
//!   the canvas origin. Coverage masks and pixel buffers live in device space.

#[cfg(not(feature = "std"))]
use alloc::string::String;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
#[cfg(not(feature = "std"))]
use alloc::{format, vec};

// In no_std builds the inherent float methods are unavailable; core_maths
// supplies them through the `CoreFloat` extension trait backed by libm.
#[cfg(not(feature = "std"))]
use core_maths::CoreFloat;

use skrifa::color::{Brush, ColorGlyph, ColorPainter, CompositeMode, PaintError, Transform};
use skrifa::outline::DrawSettings;
use skrifa::raw::tables::cpal::ColorRecord;
use skrifa::raw::types::BoundingBox;
use skrifa::{FontRef, GlyphId, MetadataProvider};
use zeno::{Fill, Mask, Origin, Transform as ZenoTransform};

use crate::color_glyph::blend::{composite, composite_layer, Premul};
use crate::color_glyph::gradient::{evaluate_brush, Rgbaf};
use crate::color_glyph::{
    resolve_palette_color, ColorRasterError, Rgba8, MAX_GLYPH_EDGE, MAX_STATE_DEPTH,
};

/// A 2D affine transform. Within the painter stack it operates in font units.
#[derive(Clone, Copy, Debug)]
struct Affine {
    xx: f32,
    xy: f32,
    yx: f32,
    yy: f32,
    dx: f32,
    dy: f32,
}

impl Affine {
    const IDENTITY: Self = Self {
        xx: 1.0,
        xy: 0.0,
        yx: 0.0,
        yy: 1.0,
        dx: 0.0,
        dy: 0.0,
    };

    fn from_skrifa(t: &Transform) -> Self {
        Self {
            xx: t.xx,
            xy: t.xy,
            yx: t.yx,
            yy: t.yy,
            dx: t.dx,
            dy: t.dy,
        }
    }

    fn is_finite(&self) -> bool {
        self.xx.is_finite()
            && self.xy.is_finite()
            && self.yx.is_finite()
            && self.yy.is_finite()
            && self.dx.is_finite()
            && self.dy.is_finite()
    }

    /// `self * other`: apply `other` first, then `self`.
    fn multiply(&self, other: &Affine) -> Affine {
        Affine {
            xx: self.xx * other.xx + self.xy * other.yx,
            xy: self.xx * other.xy + self.xy * other.yy,
            yx: self.yx * other.xx + self.yy * other.yx,
            yy: self.yx * other.xy + self.yy * other.yy,
            dx: self.xx * other.dx + self.xy * other.dy + self.dx,
            dy: self.yx * other.dx + self.yy * other.dy + self.dy,
        }
    }

    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.xx * x + self.xy * y + self.dx,
            self.yx * x + self.yy * y + self.dy,
        )
    }

    fn to_zeno(self) -> ZenoTransform {
        ZenoTransform {
            xx: self.xx,
            xy: self.xy,
            yx: self.yx,
            yy: self.yy,
            x: self.dx,
            y: self.dy,
        }
    }
}

fn invert_affine(a: &Affine) -> Option<Affine> {
    let det = a.xx * a.yy - a.xy * a.yx;
    if !det.is_finite() || det.abs() < f32::EPSILON {
        return None;
    }
    let inv_det = 1.0 / det;
    let xx = a.yy * inv_det;
    let xy = -a.xy * inv_det;
    let yx = -a.yx * inv_det;
    let yy = a.xx * inv_det;
    let dx = -(xx * a.dx + xy * a.dy);
    let dy = -(yx * a.dx + yy * a.dy);
    Some(Affine {
        xx,
        xy,
        yx,
        yy,
        dx,
        dy,
    })
}

struct Layer {
    pixels: Vec<Premul>,
    composite: CompositeMode,
}

struct ClipState {
    /// Coverage mask in device pixels, matching the canvas dimensions.
    coverage: Vec<f32>,
}

pub(super) struct RasterizingPainter<'a> {
    font: &'a FontRef<'a>,
    palette: Option<&'a [ColorRecord]>,
    foreground: Rgba8,
    width: u32,
    height: u32,
    /// Device-space origin: canvas pixel (0,0) covers the root-transformed
    /// font point `(origin_x, origin_y)`.
    origin_x: f32,
    origin_y: f32,
    /// Font units -> device pixels (no subpixel translation).
    root: Affine,
    /// Fractional pen position applied only when rasterizing coverage masks
    /// and sampling brushes, so the bitmap itself stays aligned to the pixel
    /// grid and can be reused across subpixel bins via the cache key.
    subpixel_x: f32,
    subpixel_y: f32,
    transform_stack: Vec<Affine>,
    clip_stack: Vec<ClipState>,
    layer_stack: Vec<Layer>,
    canvas: Vec<Premul>,
    location: &'a skrifa::instance::Location,
    /// First unrecoverable error encountered during painting. When set,
    /// subsequent callbacks become no-ops and `paint` returns it.
    error: Option<PaintError>,
}

impl<'a> RasterizingPainter<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        font: &'a FontRef<'a>,
        color_glyph: &'a ColorGlyph<'a>,
        root_glyph_id: GlyphId,
        palette: Option<&'a [ColorRecord]>,
        foreground: Rgba8,
        font_size: f32,
        upem: u32,
        subpixel_x: f32,
        subpixel_y: f32,
        fake_italic: bool,
        location: &'a skrifa::instance::Location,
    ) -> Result<Self, ColorRasterError> {
        let scale = font_size / upem as f32;
        if !scale.is_finite() || scale <= 0.0 {
            return Err(ColorRasterError::InvalidTransform);
        }

        // Font space is y-up; device pixels are y-down. The root transform
        // maps font units to device pixels and flips the y axis. Subpixel
        // positioning is deliberately excluded: it is applied at rasterization
        // time instead so the bounding box/canvas stay on the pixel grid.
        let scale_flip = Affine {
            xx: scale,
            xy: 0.0,
            yx: 0.0,
            yy: -scale,
            dx: 0.0,
            dy: 0.0,
        };
        let root = if fake_italic {
            // 14 degree oblique shear applied in font space, matching the
            // monochrome renderer.
            let skew = (14.0f32 * core::f32::consts::PI / 180.0).tan();
            scale_flip.multiply(&Affine {
                xx: 1.0,
                xy: skew,
                yx: 0.0,
                yy: 1.0,
                dx: 0.0,
                dy: 0.0,
            })
        } else {
            scale_flip
        };

        // Determine the device-pixel bounding box.
        //
        // `ColorGlyph::bounding_box` scales the font-unit clip box by
        // `font_size / upem` but keeps the font's y-up orientation, so the y
        // coordinates are flipped here to obtain a y-down pixel rectangle.
        // When no clip box exists, the root glyph's outline bounds (in font
        // units) are used instead; if that glyph has no outline of its own
        // (common for COLRv1 base glyphs that only reference sub-glyphs),
        // the em square is used as a conservative fallback.
        let pixel_bbox = color_glyph.bounding_box(
            skrifa::prelude::LocationRef::from(location),
            skrifa::prelude::Size::new(font_size),
        );
        let (px_min, py_min, px_max, py_max) = match pixel_bbox {
            Some(b) => (b.x_min, -b.y_max, b.x_max, -b.y_min),
            None => {
                let (fx_min, fy_min, fx_max, fy_max) = outline_bounds(
                    font,
                    root_glyph_id,
                    location,
                )?
                .unwrap_or((0.0, 0.0, upem as f32, upem as f32));
                (
                    fx_min * scale,
                    -fy_max * scale,
                    fx_max * scale,
                    -fy_min * scale,
                )
            }
        };
        if ![px_min, py_min, px_max, py_max]
            .iter()
            .all(|v| v.is_finite())
        {
            return Err(ColorRasterError::InvalidTransform);
        }

        let left = px_min.floor() as i32;
        let top = py_min.floor() as i32;
        let right = px_max.ceil() as i32;
        let bottom = py_max.ceil() as i32;
        let width = (right - left).max(0) as u32;
        let height = (bottom - top).max(0) as u32;
        if width == 0 || height == 0 {
            // Degenerate glyph: emit a minimal transparent bitmap.
            return Ok(Self {
                font,
                palette,
                foreground,
                width: 1,
                height: 1,
                origin_x: left as f32,
                origin_y: top as f32,
                root,
                subpixel_x,
                subpixel_y,
                transform_stack: Vec::new(),
                clip_stack: Vec::new(),
                layer_stack: Vec::new(),
                canvas: vec![[0.0f32; 4]; 1],
                location,
                error: None,
            });
        }
        if width > MAX_GLYPH_EDGE || height > MAX_GLYPH_EDGE {
            return Err(ColorRasterError::GlyphTooLarge);
        }

        let pixel_count = width as usize * height as usize;
        Ok(Self {
            font,
            palette,
            foreground,
            width,
            height,
            origin_x: left as f32,
            origin_y: top as f32,
            root,
            subpixel_x,
            subpixel_y,
            transform_stack: Vec::new(),
            clip_stack: Vec::new(),
            layer_stack: Vec::new(),
            canvas: vec![[0.0f32; 4]; pixel_count],
            location,
            error: None,
        })
    }

    fn current_transform(&self) -> Affine {
        self.transform_stack
            .last()
            .copied()
            .unwrap_or(Affine::IDENTITY)
    }

    fn fail(&mut self, error: PaintError) {
        if self.error.is_none() {
            self.error = Some(error);
        }
    }

    /// Full font-units -> canvas-pixel transform for the current painter
    /// state: subpixel shift, root mapping and active COLR transform, with
    /// the canvas origin removed so coordinates index the local bitmap.
    fn canvas_transform(&self) -> Affine {
        let mut device = self.root.multiply(&self.current_transform());
        device.dx += self.subpixel_x - self.origin_x;
        device.dy += self.subpixel_y - self.origin_y;
        device
    }

    /// Build the coverage mask of a glyph outline in canvas coordinates.
    fn glyph_coverage(&self, glyph_id: GlyphId) -> Option<Vec<f32>> {
        let device = self.canvas_transform();
        if !device.is_finite() {
            return None;
        }
        let mut pen = crate::color_glyph::ZenoPen::new();
        let glyph = self.font.outline_glyphs().get(glyph_id)?;
        glyph
            .draw(
                DrawSettings::unhinted(
                    skrifa::prelude::Size::unscaled(),
                    skrifa::prelude::LocationRef::from(self.location),
                ),
                &mut pen,
            )
            .ok()?;
        let path = pen.commands;
        let mut coverage = vec![0.0f32; self.width as usize * self.height as usize];
        if path.is_empty() {
            return Some(coverage);
        }

        let mut mask = Mask::new(&path);
        mask.style(Fill::NonZero)
            .transform(Some(device.to_zeno()))
            .origin(Origin::TopLeft)
            .size(self.width, self.height);
        let (data, placement) = mask.render();
        if placement.width == 0 || placement.height == 0 {
            return Some(coverage);
        }
        for row in 0..placement.height {
            for col in 0..placement.width {
                let sx = placement.left + col as i32;
                let sy = placement.top + row as i32;
                if sx < 0 || sy < 0 || sx >= self.width as i32 || sy >= self.height as i32 {
                    continue;
                }
                let src = (row * placement.width + col) as usize;
                let dst = (sy as u32 * self.width + sx as u32) as usize;
                coverage[dst] = data[src] as f32 / 255.0;
            }
        }
        Some(coverage)
    }

    /// Coverage for a clip rectangle in canvas coordinates.
    fn box_coverage(&self, bbox: &BoundingBox<f32>) -> Option<Vec<f32>> {
        let device = self.canvas_transform();
        if !device.is_finite()
            || ![bbox.x_min, bbox.y_min, bbox.x_max, bbox.y_max]
                .iter()
                .all(|v| v.is_finite())
        {
            return None;
        }
        let mut coverage = vec![0.0f32; self.width as usize * self.height as usize];
        let (x0, y0) = device.apply(bbox.x_min, bbox.y_min);
        let (x1, y1) = device.apply(bbox.x_max, bbox.y_max);
        // `device` already subtracts the canvas origin.
        let px0 = x0.min(x1);
        let px1 = x0.max(x1);
        let py0 = y0.min(y1);
        let py1 = y0.max(y1);
        let col_start = px0.floor().max(0.0) as i32;
        let col_end = (px1.ceil() as i32).min(self.width as i32);
        let row_start = py0.floor().max(0.0) as i32;
        let row_end = (py1.ceil() as i32).min(self.height as i32);
        for row in row_start..row_end {
            for col in col_start..col_end {
                coverage[(row as u32 * self.width + col as u32) as usize] = 1.0;
            }
        }
        Some(coverage)
    }

    fn push_coverage(&mut self, mut new_coverage: Vec<f32>) -> Result<(), ColorRasterError> {
        if let Some(parent) = self.clip_stack.last() {
            for (new, parent_cov) in new_coverage.iter_mut().zip(parent.coverage.iter()) {
                *new *= *parent_cov;
            }
        }
        if self.clip_stack.len() >= MAX_STATE_DEPTH {
            return Err(ColorRasterError::StateLimitExceeded);
        }
        self.clip_stack.push(ClipState {
            coverage: new_coverage,
        });
        Ok(())
    }

    /// Fill the current target (canvas or top layer) under the active clip.
    fn fill_target(&mut self, brush: &Brush<'_>, brush_transform: Option<&Affine>) {
        // Font-unit brush transform including the current painter stack.
        let brush_affine = brush_transform
            .copied()
            .map(|t| self.current_transform().multiply(&t))
            .unwrap_or_else(|| self.current_transform());
        // Full mapping font units -> canvas pixels, including subpixel shift
        // and the canvas origin offset. Its inverse takes canvas pixels back
        // into brush space for gradient parameter evaluation.
        let mut device_brush = self.root.multiply(&brush_affine);
        device_brush.dx += self.subpixel_x - self.origin_x;
        device_brush.dy += self.subpixel_y - self.origin_y;
        let Some(inverse) = invert_affine(&device_brush) else {
            // Singular brush transform: the paint is not drawable.
            self.fail(PaintError::ParseError(
                skrifa::raw::ReadError::MalformedData("singular brush transform"),
            ));
            return;
        };

        let width = self.width;
        let pixel_count = width as usize * self.height as usize;
        // Access every buffer through plain indexing to avoid overlapping
        // borrows.
        for i in 0..pixel_count {
            let coverage = match self.clip_stack.last() {
                Some(clip) => clip.coverage[i],
                None => 1.0,
            };
            if coverage <= 0.0 {
                continue;
            }
            // Canvas-local pixel coordinates.
            let device_x = (i % width as usize) as f32;
            let device_y = (i / width as usize) as f32;
            let (bx, by) = inverse.apply(device_x, device_y);
            let color = evaluate_brush(brush, bx, by, &|idx| {
                resolve_palette_color(self.palette, idx, self.foreground)
            });
            let pixel = match self.layer_stack.last_mut() {
                Some(layer) => &mut layer.pixels[i],
                None => &mut self.canvas[i],
            };
            blend_premul(pixel, &color, coverage);
        }
    }

    pub(super) fn into_image(
        mut self,
    ) -> Result<crate::color_glyph::ColorGlyphImage, ColorRasterError> {
        if let Some(error) = self.error {
            return Err(match error {
                PaintError::PaintCycleDetected => ColorRasterError::CycleDetected,
                PaintError::DepthLimitExceeded => ColorRasterError::DepthLimitExceeded,
                other => ColorRasterError::MalformedPaint(format!("{other}")),
            });
        }
        if !self.layer_stack.is_empty() {
            // Unbalanced push/pop in malformed font data.
            while let Some(layer) = self.layer_stack.pop() {
                composite_layer(layer.composite, &mut self.canvas, &layer.pixels);
            }
        }
        let width = self.width;
        let height = self.height;
        // Placement follows the swash image convention: `left` is the
        // horizontal pen offset and `top` is the positive upward distance
        // from the baseline to the bitmap's top row (callers negate it when
        // computing y-down pixel coordinates). The bitmap itself is aligned
        // to the integer pixel grid; subpixel offsets are baked into
        // coverage/brush sampling instead.
        let left = self.origin_x as i32;
        let top = -self.origin_y as i32;
        let mut data = Vec::with_capacity(self.canvas.len() * 4);
        for premul in &self.canvas {
            let a = premul[3].clamp(0.0, 1.0);
            let (r, g, b) = if a > 0.0 {
                (
                    (premul[0] / a).clamp(0.0, 1.0),
                    (premul[1] / a).clamp(0.0, 1.0),
                    (premul[2] / a).clamp(0.0, 1.0),
                )
            } else {
                (0.0, 0.0, 0.0)
            };
            data.push(round_to_u8(r));
            data.push(round_to_u8(g));
            data.push(round_to_u8(b));
            data.push(round_to_u8(a));
        }
        Ok(crate::color_glyph::ColorGlyphImage {
            data,
            width,
            height,
            left,
            top,
        })
    }
}

fn round_to_u8(v: f32) -> u8 {
    (v * 255.0 + 0.5).floor() as u8
}

/// Blend a straight-alpha brush color with coverage onto a premultiplied
/// destination pixel (source-over).
fn blend_premul(dst: &mut Premul, color: &Rgbaf, coverage: f32) {
    let alpha_s = color.a.clamp(0.0, 1.0) * coverage.clamp(0.0, 1.0);
    if alpha_s <= 0.0 {
        return;
    }
    let src = [
        color.r * alpha_s,
        color.g * alpha_s,
        color.b * alpha_s,
        alpha_s,
    ];
    *dst = composite(CompositeMode::SrcOver, src, *dst);
}

/// Outline bounds in font units, used when the COLR table has no clip box.
fn outline_bounds(
    font: &FontRef<'_>,
    glyph_id: GlyphId,
    location: &skrifa::instance::Location,
) -> Result<Option<(f32, f32, f32, f32)>, ColorRasterError> {
    let Some(glyph) = font.outline_glyphs().get(glyph_id) else {
        return Ok(None);
    };
    let mut pen = BoundsPen::default();
    glyph
        .draw(
            DrawSettings::unhinted(
                skrifa::prelude::Size::unscaled(),
                skrifa::prelude::LocationRef::from(location),
            ),
            &mut pen,
        )
        .map_err(|_| ColorRasterError::MalformedPaint(String::from("outline draw failed")))?;
    Ok(pen
        .started
        .then_some((pen.x_min, pen.y_min, pen.x_max, pen.y_max)))
}

#[derive(Default)]
struct BoundsPen {
    x_min: f32,
    y_min: f32,
    x_max: f32,
    y_max: f32,
    started: bool,
}

impl skrifa::raw::model::pen::OutlinePen for BoundsPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.include(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.include(x, y);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.include(cx, cy);
        self.include(x, y);
    }
    fn curve_to(&mut self, c0x: f32, c0y: f32, c1x: f32, c1y: f32, x: f32, y: f32) {
        self.include(c0x, c0y);
        self.include(c1x, c1y);
        self.include(x, y);
    }
    fn close(&mut self) {}
}

impl BoundsPen {
    fn include(&mut self, x: f32, y: f32) {
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        if !self.started {
            self.x_min = x;
            self.x_max = x;
            self.y_min = y;
            self.y_max = y;
            self.started = true;
        } else {
            self.x_min = self.x_min.min(x);
            self.x_max = self.x_max.max(x);
            self.y_min = self.y_min.min(y);
            self.y_max = self.y_max.max(y);
        }
    }
}

impl<'a> ColorPainter for RasterizingPainter<'a> {
    fn push_transform(&mut self, transform: Transform) {
        if !Affine::from_skrifa(&transform).is_finite() {
            // Malformed transform: keep the current transform so the stack
            // stays balanced.
            self.fail(PaintError::ParseError(
                skrifa::raw::ReadError::MalformedData("non-finite COLR transform"),
            ));
            self.transform_stack.push(self.current_transform());
            return;
        }
        let next = self
            .current_transform()
            .multiply(&Affine::from_skrifa(&transform));
        self.transform_stack.push(next);
    }

    fn pop_transform(&mut self) {
        self.transform_stack.pop();
    }

    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        if self.error.is_some() {
            return;
        }
        match self.glyph_coverage(glyph_id) {
            Some(coverage) => {
                if self.push_coverage(coverage).is_err() {
                    self.fail(PaintError::DepthLimitExceeded);
                }
            }
            None => self.fail(PaintError::GlyphNotFound(glyph_id)),
        }
    }

    fn push_clip_box(&mut self, clip_box: BoundingBox<f32>) {
        if self.error.is_some() {
            return;
        }
        match self.box_coverage(&clip_box) {
            Some(coverage) => {
                if self.push_coverage(coverage).is_err() {
                    self.fail(PaintError::DepthLimitExceeded);
                }
            }
            None => self.fail(PaintError::ParseError(
                skrifa::raw::ReadError::MalformedData("invalid clip box"),
            )),
        }
    }

    fn pop_clip(&mut self) {
        self.clip_stack.pop();
    }

    fn fill(&mut self, brush: Brush<'_>) {
        if self.error.is_some() {
            return;
        }
        self.fill_target(&brush, None);
    }

    fn fill_glyph(
        &mut self,
        glyph_id: GlyphId,
        brush_transform: Option<Transform>,
        brush: Brush<'_>,
    ) {
        if self.error.is_some() {
            return;
        }
        self.push_clip_glyph(glyph_id);
        if self.error.is_none() {
            let brush_affine = brush_transform.as_ref().map(Affine::from_skrifa);
            self.fill_target(&brush, brush_affine.as_ref());
            self.pop_clip();
        }
    }

    fn push_layer(&mut self, composite_mode: CompositeMode) {
        if self.error.is_some() {
            return;
        }
        if self.layer_stack.len() >= MAX_STATE_DEPTH {
            self.fail(PaintError::DepthLimitExceeded);
            return;
        }
        let pixels = vec![[0.0f32; 4]; self.width as usize * self.height as usize];
        self.layer_stack.push(Layer {
            pixels,
            composite: composite_mode,
        });
    }

    fn pop_layer(&mut self) {
        let Some(layer) = self.layer_stack.pop() else {
            return;
        };
        let target = match self.layer_stack.last_mut() {
            Some(parent) => parent.pixels.as_mut_slice(),
            None => self.canvas.as_mut_slice(),
        };
        composite_layer(layer.composite, target, &layer.pixels);
    }
}
