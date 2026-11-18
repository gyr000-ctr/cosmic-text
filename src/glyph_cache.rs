// SPDX-License-Identifier: MIT OR Apache-2.0

bitflags::bitflags! {
    /// Flags that change rendering
    #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
    #[repr(transparent)]
    pub struct CacheKeyFlags: u32 {
        /// Skew by 14 degrees to synthesize italic
        const FAKE_ITALIC = 1;
        /// Disable hinting
        const DISABLE_HINTING = 2;
        /// Render as a pixel font
        const PIXEL_FONT = 4;
        /// Embolden the outline to synthesize bold. The strength is packed
        /// into the upper bits, see [`CacheKeyFlags::with_synthetic_bold`]
        const FAKE_BOLD = 8;
    }
}

impl CacheKeyFlags {
    // Bits 8..=23 hold the synthetic-bold strength in EM units, quantized
    // to 1/4096, so that cache keys distinguish different strengths.
    const SYNTHETIC_BOLD_STRENGTH_SHIFT: u32 = 8;
    const SYNTHETIC_BOLD_STRENGTH_MASK: u32 = 0xFFFF << Self::SYNTHETIC_BOLD_STRENGTH_SHIFT;
    const SYNTHETIC_BOLD_STRENGTH_SCALE: f32 = 4096.0;

    /// Set [`CacheKeyFlags::FAKE_BOLD`] and pack `strength_em` (in EM units)
    /// into the flag bits
    pub fn with_synthetic_bold(self, strength_em: f32) -> Self {
        let quantized = crate::math::roundf(strength_em * Self::SYNTHETIC_BOLD_STRENGTH_SCALE)
            .max(1.0) as u32;
        let bits = (self.bits() & !Self::SYNTHETIC_BOLD_STRENGTH_MASK)
            | (quantized << Self::SYNTHETIC_BOLD_STRENGTH_SHIFT)
            | Self::FAKE_BOLD.bits();
        Self::from_bits_retain(bits)
    }

    /// The synthetic-bold strength in EM units, or `None` if
    /// [`CacheKeyFlags::FAKE_BOLD`] is not set
    pub fn synthetic_bold_strength(self) -> Option<f32> {
        if self.contains(Self::FAKE_BOLD) {
            let quantized = (self.bits() & Self::SYNTHETIC_BOLD_STRENGTH_MASK)
                >> Self::SYNTHETIC_BOLD_STRENGTH_SHIFT;
            Some(quantized as f32 / Self::SYNTHETIC_BOLD_STRENGTH_SCALE)
        } else {
            None
        }
    }
}

/// Key for building a glyph cache
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CacheKey {
    /// Font ID
    pub font_id: fontdb::ID,
    /// Glyph ID
    pub glyph_id: u16,
    /// `f32` bits of font size
    pub font_size_bits: u32,
    /// Binning of fractional X offset
    pub x_bin: SubpixelBin,
    /// Binning of fractional Y offset
    pub y_bin: SubpixelBin,
    /// Font weight
    pub font_weight: fontdb::Weight,
    /// [`CacheKeyFlags`]
    pub flags: CacheKeyFlags,
}

impl CacheKey {
    pub fn new(
        font_id: fontdb::ID,
        glyph_id: u16,
        font_size: f32,
        pos: (f32, f32),
        weight: fontdb::Weight,
        flags: CacheKeyFlags,
    ) -> (Self, i32, i32) {
        let (x, x_bin) = SubpixelBin::new(pos.0);
        let (y, y_bin) = SubpixelBin::new(pos.1);
        (
            Self {
                font_id,
                glyph_id,
                font_size_bits: font_size.to_bits(),
                x_bin,
                y_bin,
                flags,
                font_weight: weight,
            },
            x,
            y,
        )
    }
}

/// Binning of subpixel position for cache optimization
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SubpixelBin {
    Zero,
    One,
    Two,
    Three,
}

impl SubpixelBin {
    pub fn new(pos: f32) -> (i32, Self) {
        let trunc = pos as i32;
        let fract = pos - trunc as f32;

        if pos.is_sign_negative() {
            if fract > -0.125 {
                (trunc, Self::Zero)
            } else if fract > -0.375 {
                (trunc - 1, Self::Three)
            } else if fract > -0.625 {
                (trunc - 1, Self::Two)
            } else if fract > -0.875 {
                (trunc - 1, Self::One)
            } else {
                (trunc - 1, Self::Zero)
            }
        } else {
            #[allow(clippy::collapsible_else_if)]
            if fract < 0.125 {
                (trunc, Self::Zero)
            } else if fract < 0.375 {
                (trunc, Self::One)
            } else if fract < 0.625 {
                (trunc, Self::Two)
            } else if fract < 0.875 {
                (trunc, Self::Three)
            } else {
                (trunc + 1, Self::Zero)
            }
        }
    }

    pub const fn as_float(&self) -> f32 {
        match self {
            Self::Zero => 0.0,
            Self::One => 0.25,
            Self::Two => 0.5,
            Self::Three => 0.75,
        }
    }
}

#[test]
fn test_subpixel_bins() {
    // POSITIVE TESTS

    // Maps to 0.0
    assert_eq!(SubpixelBin::new(0.0), (0, SubpixelBin::Zero));
    assert_eq!(SubpixelBin::new(0.124), (0, SubpixelBin::Zero));

    // Maps to 0.25
    assert_eq!(SubpixelBin::new(0.125), (0, SubpixelBin::One));
    assert_eq!(SubpixelBin::new(0.25), (0, SubpixelBin::One));
    assert_eq!(SubpixelBin::new(0.374), (0, SubpixelBin::One));

    // Maps to 0.5
    assert_eq!(SubpixelBin::new(0.375), (0, SubpixelBin::Two));
    assert_eq!(SubpixelBin::new(0.5), (0, SubpixelBin::Two));
    assert_eq!(SubpixelBin::new(0.624), (0, SubpixelBin::Two));

    // Maps to 0.75
    assert_eq!(SubpixelBin::new(0.625), (0, SubpixelBin::Three));
    assert_eq!(SubpixelBin::new(0.75), (0, SubpixelBin::Three));
    assert_eq!(SubpixelBin::new(0.874), (0, SubpixelBin::Three));

    // Maps to 1.0
    assert_eq!(SubpixelBin::new(0.875), (1, SubpixelBin::Zero));
    assert_eq!(SubpixelBin::new(0.999), (1, SubpixelBin::Zero));
    assert_eq!(SubpixelBin::new(1.0), (1, SubpixelBin::Zero));
    assert_eq!(SubpixelBin::new(1.124), (1, SubpixelBin::Zero));

    // NEGATIVE TESTS

    // Maps to 0.0
    assert_eq!(SubpixelBin::new(-0.0), (0, SubpixelBin::Zero));
    assert_eq!(SubpixelBin::new(-0.124), (0, SubpixelBin::Zero));

    // Maps to 0.25
    assert_eq!(SubpixelBin::new(-0.125), (-1, SubpixelBin::Three));
    assert_eq!(SubpixelBin::new(-0.25), (-1, SubpixelBin::Three));
    assert_eq!(SubpixelBin::new(-0.374), (-1, SubpixelBin::Three));

    // Maps to 0.5
    assert_eq!(SubpixelBin::new(-0.375), (-1, SubpixelBin::Two));
    assert_eq!(SubpixelBin::new(-0.5), (-1, SubpixelBin::Two));
    assert_eq!(SubpixelBin::new(-0.624), (-1, SubpixelBin::Two));

    // Maps to 0.75
    assert_eq!(SubpixelBin::new(-0.625), (-1, SubpixelBin::One));
    assert_eq!(SubpixelBin::new(-0.75), (-1, SubpixelBin::One));
    assert_eq!(SubpixelBin::new(-0.874), (-1, SubpixelBin::One));

    // Maps to 1.0
    assert_eq!(SubpixelBin::new(-0.875), (-1, SubpixelBin::Zero));
    assert_eq!(SubpixelBin::new(-0.999), (-1, SubpixelBin::Zero));
    assert_eq!(SubpixelBin::new(-1.0), (-1, SubpixelBin::Zero));
    assert_eq!(SubpixelBin::new(-1.124), (-1, SubpixelBin::Zero));
}
