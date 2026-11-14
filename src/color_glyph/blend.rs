// SPDX-License-Identifier: MIT OR Apache-2.0
//! Pixel compositing and blend modes.
//!
//! Buffers inside the color rasterizer store premultiplied RGBA `f32` pixels
//! (components in `0..=1`). Compositing follows the W3C compositing and
//! blending spec (the same math referenced by the OpenType COLR table), using
//! the general `Fa`/`Fb` factor formulation so every [`CompositeMode`]
//! variant is handled.

// In no_std builds the inherent float methods are unavailable; core_maths
// supplies them through the `CoreFloat` extension trait backed by libm.
#[cfg(not(feature = "std"))]
use core_maths::CoreFloat;

use skrifa::color::CompositeMode;

/// A premultiplied RGBA pixel.
pub type Premul = [f32; 4];

/// Composite `src` over `dst` using `mode`, returning the new premultiplied
/// destination pixel.
pub fn composite(mode: CompositeMode, src: Premul, dst: Premul) -> Premul {
    let alpha_s = src[3].clamp(0.0, 1.0);
    let alpha_b = dst[3].clamp(0.0, 1.0);

    // W3C compositing factor table (`Fa`, `Fb`). SrcIn/DestIn reference the
    // other operand's alpha.
    let (fs, fb) = match mode {
        CompositeMode::Clear => (0.0, 0.0),
        CompositeMode::Src => (1.0, 0.0),
        CompositeMode::Dest => (0.0, 1.0),
        CompositeMode::SrcOver => (1.0, 1.0 - alpha_s),
        CompositeMode::DestOver => (1.0 - alpha_b, 1.0),
        CompositeMode::SrcIn => (alpha_b, 0.0),
        CompositeMode::DestIn => (0.0, alpha_s),
        CompositeMode::SrcOut => (1.0 - alpha_b, 0.0),
        CompositeMode::DestOut => (0.0, 1.0 - alpha_s),
        CompositeMode::SrcAtop => (alpha_b, 1.0 - alpha_s),
        CompositeMode::DestAtop => (1.0 - alpha_b, alpha_s),
        CompositeMode::Xor => (1.0 - alpha_b, 1.0 - alpha_s),
        // Plus and every blend mode use the source-over factor pair; their
        // special behavior lives in [`blend_channel`] / [`blend_hsl`].
        _ => (1.0, 1.0 - alpha_s),
    };

    let alpha_o = (alpha_s * fs + alpha_b * fb).clamp(0.0, 1.0);
    if alpha_o <= 0.0 {
        return [0.0; 4];
    }

    // Unpremultiplied source/destination colors.
    let cs = unpremul(src, alpha_s);
    let cb = unpremul(dst, alpha_b);

    // Non-separable HSL modes blend across all channels at once.
    let hsl_blend = blend_hsl(mode, cb, cs);

    let mut co = [0.0f32; 3];
    for channel in 0..3 {
        let blended = match hsl_blend {
            Some(hsl) => hsl[channel],
            None => blend_channel(mode, cb[channel], cs[channel]),
        };
        co[channel] = alpha_s * fs * (1.0 - alpha_b) * cs[channel]
            + alpha_s * fs * alpha_b * blended
            + alpha_b * fb * cb[channel];
        // Float rounding can overshoot slightly.
        co[channel] = co[channel].clamp(0.0, alpha_o);
    }

    [co[0], co[1], co[2], alpha_o]
}

fn unpremul(pixel: Premul, alpha: f32) -> [f32; 3] {
    if alpha <= 0.0 {
        [0.0; 3]
    } else {
        [
            (pixel[0] / alpha).clamp(0.0, 1.0),
            (pixel[1] / alpha).clamp(0.0, 1.0),
            (pixel[2] / alpha).clamp(0.0, 1.0),
        ]
    }
}

/// Separable blend function `B(Cb, Cs)` for one channel. Plus is handled
/// additively; modes without a custom function behave as source.
fn blend_channel(mode: CompositeMode, cb: f32, cs: f32) -> f32 {
    match mode {
        CompositeMode::Plus => (cb + cs).min(1.0),
        CompositeMode::Screen => screen(cb, cs),
        CompositeMode::Overlay => overlay(cb, cs),
        CompositeMode::Darken => cb.min(cs),
        CompositeMode::Lighten => cb.max(cs),
        CompositeMode::ColorDodge => color_dodge(cb, cs),
        CompositeMode::ColorBurn => color_burn(cb, cs),
        CompositeMode::HardLight => hard_light(cb, cs),
        CompositeMode::SoftLight => soft_light(cb, cs),
        CompositeMode::Difference => (cb - cs).abs(),
        CompositeMode::Exclusion => cb + cs - 2.0 * cb * cs,
        CompositeMode::Multiply => cb * cs,
        CompositeMode::HslHue
        | CompositeMode::HslSaturation
        | CompositeMode::HslColor
        | CompositeMode::HslLuminosity => {
            // Non-separable modes are applied per-pixel in the caller-facing
            // [`blend_hsl`] helper; single-channel fallback is source.
            cs
        }
        _ => cs,
    }
}

/// Apply the non-separable HSL blend modes across all three channels. Returns
/// `None` for separable modes, which the caller handles per channel.
pub fn blend_hsl(mode: CompositeMode, cb: [f32; 3], cs: [f32; 3]) -> Option<[f32; 3]> {
    match mode {
        CompositeMode::HslHue => Some(set_lum(set_sat(cs, sat(cb)), lum(cb))),
        CompositeMode::HslSaturation => Some(set_lum(set_sat(cb, sat(cs)), lum(cb))),
        CompositeMode::HslColor => Some(set_lum(cs, lum(cb))),
        CompositeMode::HslLuminosity => Some(set_lum(cb, lum(cs))),
        _ => None,
    }
}

fn screen(cb: f32, cs: f32) -> f32 {
    cb + cs - cb * cs
}

fn overlay(cb: f32, cs: f32) -> f32 {
    hard_light(cs, cb)
}

fn color_dodge(cb: f32, cs: f32) -> f32 {
    if cb <= 0.0 {
        0.0
    } else if cs >= 1.0 {
        1.0
    } else {
        1.0 - ((1.0 - cb) / cs).min(1.0)
    }
}

fn color_burn(cb: f32, cs: f32) -> f32 {
    if cb >= 1.0 {
        1.0
    } else if cs <= 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - cb) / cs).max(0.0)
    }
}

fn hard_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        multiply2(cb, 2.0 * cs)
    } else {
        screen(cb, 2.0 * cs - 1.0)
    }
}

fn soft_light(cb: f32, cs: f32) -> f32 {
    // W3C "photoshop" soft-light formulation.
    let d = if cb <= 0.25 {
        ((16.0 * cb - 12.0) * cb + 4.0) * cb
    } else {
        cb.sqrt()
    };
    if cs <= 0.5 {
        cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
    } else {
        cb + (2.0 * cs - 1.0) * (d - cb)
    }
}

fn multiply2(a: f32, b: f32) -> f32 {
    a * b
}

// --- Non-separable HSL helpers (W3C spec appendix) ---

fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn clip_color(mut c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let min = c[0].min(c[1]).min(c[2]);
    let max = c[0].max(c[1]).max(c[2]);
    if min < 0.0 {
        for channel in &mut c {
            *channel = l + (((*channel - l) * l) / (l - min));
        }
    }
    if max > 1.0 {
        for channel in &mut c {
            *channel = l + (((*channel - l) * (1.0 - l)) / (max - l));
        }
    }
    c
}

fn set_lum(mut c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    for channel in &mut c {
        *channel += d;
    }
    clip_color(c)
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn set_sat(mut c: [f32; 3], s: f32) -> [f32; 3] {
    // Index of min/max channels per the W3C formulation.
    let (mut min_i, mut mid_i, mut max_i) = (0usize, 1usize, 2usize);
    if c[min_i] > c[mid_i] {
        core::mem::swap(&mut min_i, &mut mid_i);
    }
    if c[mid_i] > c[max_i] {
        core::mem::swap(&mut mid_i, &mut max_i);
    }
    if c[min_i] > c[mid_i] {
        core::mem::swap(&mut min_i, &mut mid_i);
    }
    if c[max_i] > c[min_i] {
        c[mid_i] = ((c[mid_i] - c[min_i]) * s) / (c[max_i] - c[min_i]);
        c[max_i] = s;
    } else {
        c[mid_i] = 0.0;
        c[max_i] = 0.0;
    }
    c[min_i] = 0.0;
    c
}

/// Composite a full layer buffer onto a destination buffer using `mode`.
pub fn composite_layer(mode: CompositeMode, dst: &mut [Premul], src: &[Premul]) {
    debug_assert_eq!(dst.len(), src.len());
    for (dst_px, src_px) in dst.iter_mut().zip(src.iter()) {
        *dst_px = composite(mode, *src_px, *dst_px);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(r: f32, g: f32, b: f32, a: f32) -> Premul {
        [r * a, g * a, b * a, a]
    }

    #[test]
    fn source_over_basic() {
        let transparent = [0.0; 4];
        let red = px(1.0, 0.0, 0.0, 1.0);
        // Over transparent keeps source.
        assert_eq!(composite(CompositeMode::SrcOver, red, transparent), red);
        // Transparent source keeps destination.
        assert_eq!(composite(CompositeMode::SrcOver, transparent, red), red);
    }

    #[test]
    fn clear_erases_destination() {
        let red = px(1.0, 0.0, 0.0, 1.0);
        let out = composite(CompositeMode::Clear, red, red);
        assert_eq!(out, [0.0; 4]);
    }

    #[test]
    fn src_replaces_destination() {
        let red = px(1.0, 0.0, 0.0, 0.5);
        let blue = px(0.0, 0.0, 1.0, 1.0);
        let out = composite(CompositeMode::Src, red, blue);
        assert!((out[3] - 0.5).abs() < 1e-6);
        assert!((out[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn multiply_darkens() {
        let gray = px(0.5, 0.5, 0.5, 1.0);
        let out = composite(CompositeMode::Multiply, gray, gray);
        // 0.5 * 0.5 = 0.25, opaque
        assert!((out[0] - 0.25).abs() < 1e-6);
        assert!((out[3] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn plus_clamps_and_adds() {
        let half = px(0.6, 0.6, 0.6, 1.0);
        let out = composite(CompositeMode::Plus, half, half);
        assert!((out[0] - 1.0).abs() < 1e-6);
        assert!((out[3] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn hsl_modes_return_some() {
        let c = [0.2f32, 0.4, 0.8];
        assert!(blend_hsl(CompositeMode::HslHue, c, c).is_some());
        assert!(blend_hsl(CompositeMode::HslColor, c, c).is_some());
        assert!(blend_hsl(CompositeMode::SrcOver, c, c).is_none());
    }

    #[test]
    fn all_modes_composite_without_nan() {
        let src = px(0.3, 0.7, 0.1, 0.6);
        let dst = px(0.9, 0.2, 0.5, 0.4);
        for mode in [
            CompositeMode::Clear,
            CompositeMode::Src,
            CompositeMode::Dest,
            CompositeMode::SrcOver,
            CompositeMode::DestOver,
            CompositeMode::SrcIn,
            CompositeMode::DestIn,
            CompositeMode::SrcOut,
            CompositeMode::DestOut,
            CompositeMode::SrcAtop,
            CompositeMode::DestAtop,
            CompositeMode::Xor,
            CompositeMode::Plus,
            CompositeMode::Screen,
            CompositeMode::Overlay,
            CompositeMode::Darken,
            CompositeMode::Lighten,
            CompositeMode::ColorDodge,
            CompositeMode::ColorBurn,
            CompositeMode::HardLight,
            CompositeMode::SoftLight,
            CompositeMode::Difference,
            CompositeMode::Exclusion,
            CompositeMode::Multiply,
            CompositeMode::HslHue,
            CompositeMode::HslSaturation,
            CompositeMode::HslColor,
            CompositeMode::HslLuminosity,
        ] {
            let out = composite(mode, src, dst);
            for v in out {
                assert!(v.is_finite(), "mode {mode:?} produced non-finite value");
                assert!((0.0..=1.0).contains(&v));
            }
        }
    }
}
