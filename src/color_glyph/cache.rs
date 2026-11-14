// SPDX-License-Identifier: MIT OR Apache-2.0
//! Bounded cache for rasterized COLR/CPAL color glyph bitmaps.

#[cfg(not(feature = "std"))]
use alloc::collections::VecDeque;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
#[cfg(feature = "std")]
use std::collections::VecDeque;

use crate::color_glyph::{rasterize, PaletteSelector};
use crate::{CacheKey, Color, FontSystem, HashMap};

/// Default maximum number of glyph bitmaps retained by [`ColorGlyphCache`].
pub const DEFAULT_CACHE_ENTRY_LIMIT: usize = 512;

/// Default upper bound on total bitmap memory, in bytes (32 MiB).
pub const DEFAULT_CACHE_BYTE_LIMIT: usize = 32 * 1024 * 1024;

/// Errors produced while rasterizing a color glyph. A caller receiving any of
/// these should fall back to the monochrome outline (or a replacement glyph).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColorRasterError {
    /// The requested font is not present in the font system.
    FontNotFound,
    /// The glyph has no COLR table entry, so color rasterization does not
    /// apply and the normal outline path should be used.
    NotAColorGlyph,
    /// A paint graph cycle was detected.
    CycleDetected,
    /// The paint graph is deeper than the allowed traversal limit.
    DepthLimitExceeded,
    /// A gradient or other paint structure contained invalid data.
    InvalidGradient,
    /// A transformation matrix was non-finite or otherwise unusable.
    InvalidTransform,
    /// The transformed glyph bounds exceed [`MAX_GLYPH_EDGE`] on an edge.
    GlyphTooLarge,
    /// The painter ran out of clip/layer stack space.
    StateLimitExceeded,
    /// The paint graph referenced data that failed to parse.
    MalformedPaint(alloc::string::String),
}

impl core::fmt::Display for ColorRasterError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::FontNotFound => f.write_str("font not found"),
            Self::NotAColorGlyph => f.write_str("glyph has no COLR paint graph"),
            Self::CycleDetected => f.write_str("COLR paint cycle detected"),
            Self::DepthLimitExceeded => f.write_str("COLR paint graph depth limit exceeded"),
            Self::InvalidGradient => f.write_str("invalid gradient in COLR paint graph"),
            Self::InvalidTransform => f.write_str("invalid transform in COLR paint graph"),
            Self::GlyphTooLarge => f.write_str("color glyph exceeds maximum raster size"),
            Self::StateLimitExceeded => f.write_str("COLR clip/layer state limit exceeded"),
            Self::MalformedPaint(msg) => write!(f, "malformed COLR paint: {msg}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ColorRasterError {}

/// A rasterized color glyph: premultiplied-independent RGBA8 bitmap plus its
/// placement relative to the glyph origin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorGlyphImage {
    /// Pixel data, `width * height * 4` bytes, row-major RGBA (not
    /// premultiplied after finalization).
    pub data: Vec<u8>,
    /// Bitmap width in pixels.
    pub width: u32,
    /// Bitmap height in pixels.
    pub height: u32,
    /// Horizontal offset from the pen position to the left edge of the bitmap.
    pub left: i32,
    /// Vertical offset from the pen position to the top edge of the bitmap,
    /// following the same convention as swash image placement.
    pub top: i32,
}

impl ColorGlyphImage {
    fn byte_len(&self) -> usize {
        self.data.len()
    }
}

/// Full key under which a color bitmap is cached. This covers the font,
/// glyph, size, subpixel position, weight, render flags, palette selection
/// and foreground color, so distinct renders never share a bitmap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ColorCacheKey {
    /// Monochrome-compatible cache key (font, glyph, size, bins, flags...).
    pub cache_key: CacheKey,
    /// Resolved CPAL palette index. `0` is the font default.
    pub palette_index: u16,
    /// Foreground text color, used for `0xFFFF` palette entries.
    pub foreground: Color,
}

struct Entry {
    image: Option<ColorGlyphImage>,
}

/// Cache of rasterized color glyphs with bounded entry count and total
/// memory. Create one per application (like `SwashCache`) and reuse it across
/// frames so the COLR paint graph is only traversed once per distinct key.
pub struct ColorGlyphCache {
    entries: HashMap<ColorCacheKey, Entry>,
    /// Insertion-order queue for eviction.
    order: VecDeque<ColorCacheKey>,
    entry_limit: usize,
    byte_limit: usize,
    total_bytes: usize,
}

impl core::fmt::Debug for ColorGlyphCache {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ColorGlyphCache")
            .field("entries", &self.entries.len())
            .field("total_bytes", &self.total_bytes)
            .field("entry_limit", &self.entry_limit)
            .field("byte_limit", &self.byte_limit)
            .finish()
    }
}

impl Default for ColorGlyphCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ColorGlyphCache {
    /// Create a cache with the default entry and memory limits.
    pub fn new() -> Self {
        Self::with_limits(DEFAULT_CACHE_ENTRY_LIMIT, DEFAULT_CACHE_BYTE_LIMIT)
    }

    /// Create a cache with explicit maximum entry count and total bitmap byte
    /// budget.
    pub fn with_limits(entry_limit: usize, byte_limit: usize) -> Self {
        Self {
            entries: HashMap::default(),
            order: VecDeque::with_capacity(entry_limit.min(1024)),
            entry_limit: entry_limit.max(1),
            byte_limit,
            total_bytes: 0,
        }
    }

    /// Number of cached bitmaps.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Total bytes currently occupied by cached bitmaps.
    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    /// Remove all cached bitmaps.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.total_bytes = 0;
    }

    fn make_key(cache_key: CacheKey, palette: PaletteSelector, foreground: Color) -> ColorCacheKey {
        ColorCacheKey {
            cache_key,
            palette_index: palette.resolved_index(),
            foreground,
        }
    }

    /// Rasterize a color glyph, caching and reusing the result.
    ///
    /// Returns `Ok(None)` when the glyph is not a COLR color glyph (callers
    /// should then use the monochrome path), and `Err` for malformed fonts or
    /// oversized renders that must fall back.
    ///
    /// # Errors
    ///
    /// Returns a [`ColorRasterError`] if the COLR paint graph cannot be
    /// traversed (cycles, depth overflow, malformed tables) or the resulting
    /// bitmap would exceed the size limits.
    pub fn get(
        &mut self,
        font_system: &mut FontSystem,
        cache_key: CacheKey,
        palette: PaletteSelector,
        foreground: Color,
    ) -> Result<Option<&ColorGlyphImage>, ColorRasterError> {
        let key = Self::make_key(cache_key, palette, foreground);
        if !self.entries.contains_key(&key) {
            let result = match rasterize(font_system, cache_key, palette, foreground) {
                Ok(image) => Some(image),
                Err(ColorRasterError::NotAColorGlyph) => None,
                Err(other) => return Err(other),
            };
            self.insert(key, result);
        }
        Ok(self.entries.get(&key).and_then(|e| e.image.as_ref()))
    }

    /// Enumerate the RGBA pixels of a color glyph, mirroring
    /// [`SwashCache::with_pixels`](crate::SwashCache::with_pixels).
    ///
    /// Coordinates are relative to the glyph origin and suitable for passing
    /// straight to a rectangle-based renderer. Returns:
    /// * `Ok(true)` if a color glyph was painted,
    /// * `Ok(false)` if the glyph has no COLR table (the caller should fall
    ///   back to its monochrome path),
    /// * `Err` for malformed data or oversized glyphs, which likewise warrant
    ///   a monochrome/replacement fallback.
    ///
    /// # Errors
    ///
    /// See [`ColorGlyphCache::get`] for the error conditions.
    pub fn with_pixels<F: FnMut(i32, i32, Color)>(
        &mut self,
        font_system: &mut FontSystem,
        cache_key: CacheKey,
        palette: PaletteSelector,
        foreground: Color,
        mut f: F,
    ) -> Result<bool, ColorRasterError> {
        let Some(image) = self.get(font_system, cache_key, palette, foreground)? else {
            return Ok(false);
        };
        for row in 0..image.height {
            for col in 0..image.width {
                let i = ((row * image.width + col) * 4) as usize;
                f(
                    image.left + col as i32,
                    // Match `SwashCache::with_pixels`: `top` is the upward
                    // distance from the baseline, so y-down rows negate it.
                    -image.top + row as i32,
                    Color::rgba(
                        image.data[i],
                        image.data[i + 1],
                        image.data[i + 2],
                        image.data[i + 3],
                    ),
                );
            }
        }
        Ok(true)
    }

    fn insert(&mut self, key: ColorCacheKey, image: Option<ColorGlyphImage>) {
        // Negative results (no image) occupy a tiny entry and are cached too,
        // avoiding repeated paint-graph lookups for non-color glyphs.
        let added = image.as_ref().map_or(0, ColorGlyphImage::byte_len);
        self.entries.insert(key, Entry { image });
        self.order.push_back(key);
        self.total_bytes = self.total_bytes.saturating_add(added);
        self.evict();
    }

    fn evict(&mut self) {
        // If the most recently inserted entry alone exceeds the byte budget,
        // keep it: evicting it would leave an empty cache that re-rasterizes
        // the same glyph on every frame. The entry-count bound still applies
        // and it gets replaced as soon as something newer is inserted.
        if self.entries.len() == 1 && self.total_bytes > self.byte_limit {
            return;
        }
        // FIFO eviction keeps the bookmark allocation-free on the hot path
        // while guaranteeing both bounds hold.
        while self.entries.len() > self.entry_limit || self.total_bytes > self.byte_limit {
            let Some(victim_key) = self.order.pop_front() else {
                break;
            };
            if let Some(victim) = self.entries.remove(&victim_key) {
                if let Some(image) = &victim.image {
                    self.total_bytes = self.total_bytes.saturating_sub(image.byte_len());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_image(side: u32, stamp_byte: u8) -> ColorGlyphImage {
        ColorGlyphImage {
            data: vec![stamp_byte; (side * side * 4) as usize],
            width: side,
            height: side,
            left: 0,
            top: 0,
        }
    }

    fn key(n: u16) -> ColorCacheKey {
        ColorCacheKey {
            cache_key: CacheKey {
                font_id: fontdb::ID::dummy(),
                glyph_id: n,
                font_size_bits: 16f32.to_bits(),
                x_bin: crate::SubpixelBin::Zero,
                y_bin: crate::SubpixelBin::Zero,
                font_weight: fontdb::Weight::NORMAL,
                flags: crate::CacheKeyFlags::empty(),
            },
            palette_index: 0,
            foreground: Color::rgb(0, 0, 0),
        }
    }

    #[test]
    fn entry_limit_is_enforced() {
        let mut cache = ColorGlyphCache::with_limits(2, usize::MAX);
        cache.insert(key(1), Some(fake_image(2, 1)));
        cache.insert(key(2), Some(fake_image(2, 2)));
        cache.insert(key(3), Some(fake_image(2, 3)));
        assert_eq!(cache.len(), 2);
        assert!(cache.entries.contains_key(&key(2)));
        assert!(cache.entries.contains_key(&key(3)));
        assert!(!cache.entries.contains_key(&key(1)));
    }

    #[test]
    fn byte_limit_is_enforced() {
        // Each 4x4 RGBA image is 64 bytes. Budget holds two.
        let mut cache = ColorGlyphCache::with_limits(100, 130);
        cache.insert(key(1), Some(fake_image(4, 1)));
        cache.insert(key(2), Some(fake_image(4, 2)));
        cache.insert(key(3), Some(fake_image(4, 3)));
        assert!(cache.total_bytes() <= 130);
        assert!(cache.len() <= 2);
    }

    #[test]
    fn oversized_single_entry_does_not_loop_forever() {
        let mut cache = ColorGlyphCache::with_limits(100, 8);
        cache.insert(key(1), Some(fake_image(4, 1)));
        assert_eq!(cache.len(), 1);
        assert!(cache.total_bytes() >= 64);
    }

    #[test]
    fn negative_results_are_cached_and_bounded() {
        let mut cache = ColorGlyphCache::with_limits(1, usize::MAX);
        cache.insert(key(1), None);
        assert_eq!(cache.len(), 1);
        cache.insert(key(2), None);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.total_bytes(), 0);
    }

    #[test]
    fn clear_resets_state() {
        let mut cache = ColorGlyphCache::new();
        cache.insert(key(1), Some(fake_image(2, 1)));
        assert!(!cache.is_empty());
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.total_bytes(), 0);
    }
}
