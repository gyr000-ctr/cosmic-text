// SPDX-License-Identifier: MIT OR Apache-2.0

use core::fmt::Display;

use core::ops::Range;

use crate::{math, CacheKey, CacheKeyFlags, Color, GlyphDecorationData, LayoutRun};
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

#[cfg(not(feature = "std"))]
use core_maths::CoreFloat;
/// A laid out glyph
#[derive(Clone, Debug)]
pub struct LayoutGlyph {
    /// Start index of cluster in original line
    pub start: usize,
    /// End index of cluster in original line
    pub end: usize,
    /// Font size of the glyph
    pub font_size: f32,
    /// Font weight of the glyph
    pub font_weight: fontdb::Weight,
    /// Line height of the glyph, will override buffer setting
    pub line_height_opt: Option<f32>,
    /// Font id of the glyph
    pub font_id: fontdb::ID,
    /// Font id of the glyph
    pub glyph_id: u16,
    /// X offset of hitbox
    pub x: f32,
    /// Y offset of hitbox
    pub y: f32,
    /// Width of hitbox
    pub w: f32,
    /// Ascender of the glyph in pixels
    ///
    /// This is the distance from the baseline to the top of the glyph
    pub ascender: f32,
    /// Descender of the glyph in pixels
    ///
    /// This is the distance from the baseline to the bottom of the glyph
    pub descender: f32,
    /// Unicode `BiDi` embedding level, character is left-to-right if `level` is divisible by 2
    pub level: unicode_bidi::Level,
    /// X offset in line
    ///
    /// If you are dealing with physical coordinates, use [`Self::physical`] to obtain a
    /// [`PhysicalGlyph`] for rendering.
    ///
    /// This offset is useful when you are dealing with logical units and you do not care or
    /// cannot guarantee pixel grid alignment. For instance, when you want to use the glyphs
    /// for vectorial text, apply linear transformations to the layout, etc.
    pub x_offset: f32,
    /// Y offset in line
    ///
    /// If you are dealing with physical coordinates, use [`Self::physical`] to obtain a
    /// [`PhysicalGlyph`] for rendering.
    ///
    /// This offset is useful when you are dealing with logical units and you do not care or
    /// cannot guarantee pixel grid alignment. For instance, when you want to use the glyphs
    /// for vectorial text, apply linear transformations to the layout, etc.
    pub y_offset: f32,
    /// Optional color override
    pub color_opt: Option<Color>,
    /// Metadata from `Attrs`
    pub metadata: usize,
    /// [`CacheKeyFlags`]
    pub cache_key_flags: CacheKeyFlags,
}

/// A span of consecutive glyphs sharing the same text decoration.
#[derive(Clone, Debug, PartialEq)]
pub struct DecorationSpan {
    /// Range of glyph indices in `LayoutLine::glyphs` covered by this span
    pub glyph_range: Range<usize>,
    /// The decoration config and metrics
    pub data: GlyphDecorationData,
    /// Fallback color from the first glyph's `color_opt`
    pub color_opt: Option<Color>,
    /// Font size from the first glyph (used to scale EM-unit metrics)
    pub font_size: f32,
}

impl DecorationSpan {
    /// The horizontal extent of this span's glyphs within `run`, which must
    /// be the layout run containing the span. Empty if the span covers no
    /// glyphs.
    pub fn x_range(&self, run: &LayoutRun) -> Range<f32> {
        // Compute the extent as min/max over all glyphs, not first/last,
        // because RTL paragraphs store glyphs in right-to-left order.
        let mut x_min = f32::INFINITY;
        let mut x_max = f32::NEG_INFINITY;
        for glyph in &run.glyphs[self.glyph_range.clone()] {
            x_min = x_min.min(glyph.x);
            x_max = x_max.max(glyph.x + glyph.w);
        }

        if x_max > x_min {
            x_min..x_max
        } else {
            0.0..0.0
        }
    }
}

#[derive(Clone, Debug)]
pub struct PhysicalGlyph {
    /// Cache key, see [`CacheKey`]
    pub cache_key: CacheKey,
    /// Integer component of X offset in line
    pub x: i32,
    /// Integer component of Y offset in line
    pub y: i32,
}

impl LayoutGlyph {
    pub fn physical(&self, offset: (f32, f32), scale: f32) -> PhysicalGlyph {
        let x_offset = self.font_size * self.x_offset;
        let y_offset = self.font_size * self.y_offset;

        let (cache_key, x, y) = CacheKey::new(
            self.font_id,
            self.glyph_id,
            self.font_size * scale,
            (
                (self.x + x_offset) * scale + offset.0,
                math::truncf((self.y - y_offset) * scale + offset.1), // Hinting in Y axis
            ),
            self.font_weight,
            self.cache_key_flags,
        );

        PhysicalGlyph { cache_key, x, y }
    }
}

/// A line of laid out glyphs
#[derive(Clone, Debug)]
pub struct LayoutLine {
    /// Width of the line
    pub w: f32,
    /// Maximum ascent of the glyphs in line
    pub max_ascent: f32,
    /// Maximum descent of the glyphs in line
    pub max_descent: f32,
    /// Maximum line height of any spans in line
    pub line_height_opt: Option<f32>,
    /// Glyphs in line
    pub glyphs: Vec<LayoutGlyph>,
    /// Text decoration spans covering ranges of glyphs
    pub decorations: Vec<DecorationSpan>,
}

/// Wrapping mode
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum Wrap {
    /// No wrapping
    None,
    /// Wraps at a glyph level
    Glyph,
    /// Wraps at the word level
    Word,
    /// Wraps at the word level, or fallback to glyph level if a word can't fit on a line by itself
    WordOrGlyph,
}

impl Display for Wrap {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::None => write!(f, "No Wrap"),
            Self::Word => write!(f, "Word Wrap"),
            Self::WordOrGlyph => write!(f, "Word Wrap or Character"),
            Self::Glyph => write!(f, "Character"),
        }
    }
}

/// Align or justify
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum Align {
    Left,
    Right,
    Center,
    Justified,
    End,
}

impl Display for Align {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Left => write!(f, "Left"),
            Self::Right => write!(f, "Right"),
            Self::Center => write!(f, "Center"),
            Self::Justified => write!(f, "Justified"),
            Self::End => write!(f, "End"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Ellipsize {
    /// No Ellipsizing
    #[default]
    None,
    /// Ellipsizes the start of the last visual line that fits within the `EllipsizeHeightLimit`
    Start(EllipsizeHeightLimit),
    /// Ellipsizes the middle of the last visual line that fits within the `EllipsizeHeightLimit`.
    Middle(EllipsizeHeightLimit),
    /// Ellipsizes the end of the last visual line that fits within the `EllipsizeHeightLimit`.
    End(EllipsizeHeightLimit),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EllipsizeHeightLimit {
    /// Number of lines to show before ellipsizing the rest. Only works if `Wrap` is NOT set to
    /// `Wrap::None`. Otherwise, it will be ignored and the behavior will be the same as `Lines(1)`
    Lines(usize),
    /// Ellipsizes the last line that fits within the given height limit. If `Wrap` is set to
    /// `Wrap::None`, the behavior will be the same as `Lines(1)`
    Height(f32),
}

/// Metrics hinting strategy
#[derive(Debug, Eq, PartialEq, Clone, Copy, Default)]
pub enum Hinting {
    /// No metrics hinting.
    ///
    /// Glyphs will have subpixel coordinates.
    ///
    /// This is the default.
    #[default]
    Disabled,

    /// Metrics hinting.
    ///
    /// Glyphs will be snapped to integral coordinates in the X-axis during layout.
    /// This can improve readability for smaller text and/or low-DPI screens.
    ///
    /// However, in order to get the right effect, you must use physical coordinates
    /// during layout and avoid further scaling when rendering. Otherwise, the rounding
    /// errors can accumulate and glyph distances may look erratic.
    ///
    /// In other words, metrics hinting makes layouting dependent of the target
    /// resolution.
    Enabled,
}

/// Paragraph indentation, in pixels.
///
/// Indentation applies per logical paragraph (a [`crate::BufferLine`]): the
/// first visual line uses `first_line`, while every soft-wrapped continuation
/// line uses `hanging`. Soft wraps therefore never repeat the first-line
/// indent.
///
/// For LTR paragraphs a positive value shifts the affected line's start edge
/// to the right; for RTL paragraphs it shifts the start edge to the left. The
/// shift is mirrored automatically, so the same value gives the typographically
/// correct result in both directions.
///
/// Both values must be finite and non-negative. Indentation is validated with
/// [`Indent::validate`]; use [`crate::BufferLine::set_indent`] and
/// [`crate::Buffer::set_indent`] to apply it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Indent {
    /// Extra indentation of the first visual line of the paragraph, in pixels.
    pub first_line: f32,
    /// Indentation of every soft-wrapped continuation line, in pixels.
    pub hanging: f32,
}

impl Indent {
    /// No indentation.
    pub const NONE: Self = Self {
        first_line: 0.0,
        hanging: 0.0,
    };

    /// Create an indentation descriptor.
    ///
    /// The values are not validated here; pass the result to
    /// [`Self::validate`] (or to a setter that validates) before use.
    pub const fn new(first_line: f32, hanging: f32) -> Self {
        Self {
            first_line,
            hanging,
        }
    }

    /// Indentation with only a first-line indent.
    pub const fn first_line(first_line: f32) -> Self {
        Self {
            first_line,
            hanging: 0.0,
        }
    }

    /// Indentation with only a hanging indent.
    pub const fn hanging(hanging: f32) -> Self {
        Self {
            first_line: 0.0,
            hanging,
        }
    }

    /// True if neither field moves any line.
    pub const fn is_zero(&self) -> bool {
        self.first_line == 0.0 && self.hanging == 0.0
    }

    /// Validate the values against an optional paragraph width.
    ///
    /// `width_opt` is the available text width (`None` means the paragraph is
    /// not width-constrained).
    ///
    /// # Errors
    ///
    /// Returns [`IndentError::Negative`] if either value is negative, NaN, or
    /// otherwise non-finite, and [`IndentError::TooWide`] if the indentation
    /// of the first line or of a continuation line would leave no room for
    /// text within `width_opt`.
    pub fn validate(&self, width_opt: Option<f32>) -> Result<(), IndentError> {
        if !self.first_line.is_finite() || !self.hanging.is_finite() {
            return Err(IndentError::Negative {
                first_line: self.first_line,
                hanging: self.hanging,
            });
        }
        if self.first_line < 0.0 || self.hanging < 0.0 {
            return Err(IndentError::Negative {
                first_line: self.first_line,
                hanging: self.hanging,
            });
        }
        // Zero indentation is always valid, including on a zero-width buffer.
        if self.is_zero() {
            return Ok(());
        }
        if let Some(width) = width_opt {
            if self.first_line >= width || self.hanging >= width {
                return Err(IndentError::TooWide {
                    indent: *self,
                    width,
                });
            }
        }
        Ok(())
    }

    /// Start-edge shift of the line at visual index `line_index` within the
    /// paragraph. Positive for an outward (start-edge) shift.
    pub(crate) fn line_shift(&self, line_index: usize) -> f32 {
        if line_index == 0 {
            self.first_line
        } else {
            self.hanging
        }
    }
}

/// An error returned when paragraph indentation is invalid.
///
/// The two variants distinguish an out-of-range value (negative or non-finite
/// indentation) from an indentation that does not fit inside the paragraph
/// width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IndentError {
    /// At least one indentation value is negative or not finite.
    Negative {
        /// The rejected first-line value.
        first_line: f32,
        /// The rejected hanging value.
        hanging: f32,
    },
    /// The indentation leaves no room for text within the paragraph width.
    TooWide {
        /// The indentation that was rejected.
        indent: Indent,
        /// The paragraph width it did not fit into.
        width: f32,
    },
}

impl core::fmt::Display for IndentError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Negative {
                first_line,
                hanging,
            } => write!(
                f,
                "indentation values must be finite and non-negative, got first_line={first_line}, hanging={hanging}"
            ),
            Self::TooWide { indent, width } => write!(
                f,
                "indentation (first_line={}, hanging={}) must be smaller than the paragraph width {width}",
                indent.first_line, indent.hanging
            ),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for IndentError {}
