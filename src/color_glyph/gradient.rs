// SPDX-License-Identifier: MIT OR Apache-2.0
//! Gradient parameter evaluation for the `COLRv1` brush types in the
//! `skrifa::color` module.
//!
//! Color stops are interpolated in gamma-encoded sRGB space (the COLR
//! default) with premultiplied alpha. The three [`Extend`] modes are applied
//! to the normalized gradient parameter before the stop lookup.

// In no_std builds the inherent float methods are unavailable; core_maths
// supplies them through the `CoreFloat` extension trait backed by libm.
#[cfg(not(feature = "std"))]
use core_maths::CoreFloat;

use skrifa::color::{Brush, ColorStop, Extend};

use super::Rgba8;

/// A fully resolved solid color in straight alpha, components 0..=1.
#[derive(Clone, Copy, Debug)]
pub struct Rgbaf {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgbaf {
    pub fn from_rgba8(c: Rgba8) -> Self {
        Self {
            r: c.r as f32 / 255.0,
            g: c.g as f32 / 255.0,
            b: c.b as f32 / 255.0,
            a: c.a as f32 / 255.0,
        }
    }
}

/// Resolve a solid brush to a straight-alpha color, applying the brush alpha
/// multiplier.
pub fn solid_color(palette_index: u16, alpha: f32, resolve: impl Fn(u16) -> Rgba8) -> Rgbaf {
    let base = Rgbaf::from_rgba8(resolve(palette_index));
    let alpha = sanitize_alpha(alpha);
    Rgbaf {
        r: base.r,
        g: base.g,
        b: base.b,
        a: base.a * alpha,
    }
}

fn sanitize_alpha(alpha: f32) -> f32 {
    if alpha.is_finite() {
        alpha.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Apply the COLR extend modes to a normalized gradient parameter.
fn extend_parameter(t: f32, extend: Extend) -> f32 {
    if !t.is_finite() {
        return 0.0;
    }
    match extend {
        Extend::Pad => t.clamp(0.0, 1.0),
        Extend::Repeat => {
            let t = t - t.floor();
            if t < 0.0 {
                t + 1.0
            } else {
                t
            }
        }
        Extend::Reflect => {
            let doubled = t * 0.5;
            let folded = (doubled - doubled.floor()).abs() * 2.0;
            1.0 - (folded - 1.0).abs()
        }
        // Unknown extend values come from malformed fonts; pad is the safe
        // deterministic fallback.
        Extend::Unknown => t.clamp(0.0, 1.0),
    }
}

/// Interpolate the color stops at normalized parameter `t` (already extended).
fn sample_stops(stops: &[ColorStop], t: f32, resolve: &impl Fn(u16) -> Rgba8) -> Rgbaf {
    if stops.is_empty() {
        return Rgbaf {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        };
    }
    if stops.len() == 1 {
        let stop = stops[0];
        return solid_color(stop.palette_index, stop.alpha, resolve);
    }

    if t <= stops[0].offset {
        let stop = stops[0];
        return solid_color(stop.palette_index, stop.alpha, resolve);
    }
    let last = stops.len() - 1;
    if t >= stops[last].offset {
        let stop = stops[last];
        return solid_color(stop.palette_index, stop.alpha, resolve);
    }

    // Stops are normalized/sorted by skrifa; find the surrounding pair.
    let mut upper = 1;
    while upper < last && stops[upper].offset < t {
        upper += 1;
    }
    let lower = upper - 1;
    let s0 = stops[lower];
    let s1 = stops[upper];
    let span = s1.offset - s0.offset;
    let local_t = if span > 0.0 {
        ((t - s0.offset) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let c0 = solid_color(s0.palette_index, s0.alpha, resolve);
    let c1 = solid_color(s1.palette_index, s1.alpha, resolve);
    interpolate_premul(c0, c1, local_t)
}

/// sRGB-space interpolation with premultiplied alpha.
fn interpolate_premul(c0: Rgbaf, c1: Rgbaf, t: f32) -> Rgbaf {
    let a = c0.a + (c1.a - c0.a) * t;
    if a <= 0.0 {
        return Rgbaf {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        };
    }
    let mix = |x: f32, y: f32| {
        // Premultiply, interpolate, then unpremultiply.
        let px = x * c0.a;
        let py = y * c1.a;
        let p = px + (py - px) * t;
        (p / a).clamp(0.0, 1.0)
    };
    Rgbaf {
        r: mix(c0.r, c1.r),
        g: mix(c0.g, c1.g),
        b: mix(c0.b, c1.b),
        a,
    }
}

/// Parameter of a linear gradient at point `(x, y)` in the brush's
/// coordinate space.
pub fn linear_parameter(p0: (f32, f32), p1: (f32, f32), x: f32, y: f32) -> Option<f32> {
    let dx = p1.0 - p0.0;
    let dy = p1.1 - p0.1;
    let len_sq = dx * dx + dy * dy;
    if !len_sq.is_finite() || len_sq <= f32::EPSILON {
        return None;
    }
    Some(((x - p0.0) * dx + (y - p0.1) * dy) / len_sq)
}

/// Parameter of a radial gradient at point `(x, y)`. Negative start radii are
/// handled by clamping the color line at 0 per the COLR spec.
pub fn radial_parameter(
    c0: (f32, f32),
    r0: f32,
    c1: (f32, f32),
    r1: f32,
    x: f32,
    y: f32,
) -> Option<f32> {
    if !r0.is_finite() || !r1.is_finite() {
        return None;
    }
    // Vector from c0 to c1.
    let fx = c1.0 - c0.0;
    let fy = c1.1 - c0.1;
    let dx = x - c0.0;
    let dy = y - c0.1;
    let a = fx * fx + fy * fy - (r1 - r0) * (r1 - r0);
    let b = 2.0 * (dx * fx + dy * fy - r0 * (r1 - r0));
    let c = dx * dx + dy * dy - r0 * r0;

    let t = if a.abs() <= f32::EPSILON {
        // Linear case (concentric circles or collinear degenerate cone).
        if b.abs() <= f32::EPSILON {
            return None;
        }
        -c / b
    } else {
        let disc = b * b - 4.0 * a * c;
        if !disc.is_finite() || disc < 0.0 {
            return None;
        }
        let sqrt_disc = disc.sqrt();
        let t1 = (-b - sqrt_disc) / (2.0 * a);
        let t2 = (-b + sqrt_disc) / (2.0 * a);
        // Choose the nearest non-negative intersection.
        match (t1 >= 0.0, t2 >= 0.0) {
            (true, true) => t1.min(t2),
            (true, false) => t1,
            (false, true) => t2,
            (false, false) => return None,
        }
    };
    Some(t)
}

/// Parameter of a sweep (conical) gradient. Angles are clockwise in degrees,
/// as normalized by skrifa.
pub fn sweep_parameter(
    center: (f32, f32),
    start_angle: f32,
    end_angle: f32,
    x: f32,
    y: f32,
) -> Option<f32> {
    let span = end_angle - start_angle;
    if !span.is_finite() || span.abs() <= f32::EPSILON {
        return None;
    }
    // Clockwise angle in degrees (atan2 gives radians, y-up).
    let angle = -((y - center.1).atan2(x - center.0)) * 180.0 / core::f32::consts::PI;
    Some((angle - start_angle) / span)
}

/// Evaluate any brush at a point in the brush coordinate space, returning a
/// straight-alpha color.
pub fn evaluate_brush(brush: &Brush<'_>, x: f32, y: f32, resolve: &impl Fn(u16) -> Rgba8) -> Rgbaf {
    match brush {
        Brush::Solid {
            palette_index,
            alpha,
        } => solid_color(*palette_index, *alpha, resolve),
        Brush::LinearGradient {
            p0,
            p1,
            color_stops,
            extend,
        } => {
            let t = linear_parameter((p0.x, p0.y), (p1.x, p1.y), x, y).unwrap_or(0.0);
            let t = extend_parameter(t, *extend);
            sample_stops(color_stops, t, resolve)
        }
        Brush::RadialGradient {
            c0,
            r0,
            c1,
            r1,
            color_stops,
            extend,
        } => {
            let t = radial_parameter((c0.x, c0.y), *r0, (c1.x, c1.y), *r1, x, y).unwrap_or(0.0);
            let t = extend_parameter(t, *extend);
            sample_stops(color_stops, t, resolve)
        }
        Brush::SweepGradient {
            c0,
            start_angle,
            end_angle,
            color_stops,
            extend,
        } => {
            let t = sweep_parameter((c0.x, c0.y), *start_angle, *end_angle, x, y).unwrap_or(0.0);
            let t = extend_parameter(t, *extend);
            sample_stops(color_stops, t, resolve)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(_index: u16) -> Rgba8 {
        Rgba8 {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }
    }

    #[test]
    fn extend_modes() {
        assert!((extend_parameter(-0.5, Extend::Pad) - 0.0).abs() < 1e-6);
        assert!((extend_parameter(1.5, Extend::Pad) - 1.0).abs() < 1e-6);

        assert!((extend_parameter(1.25, Extend::Repeat) - 0.25).abs() < 1e-6);
        assert!((extend_parameter(-0.25, Extend::Repeat) - 0.75).abs() < 1e-6);

        // Reflect: 1.25 -> 0.75, 1.75 -> 0.25
        assert!((extend_parameter(1.25, Extend::Reflect) - 0.75).abs() < 1e-6);
        assert!((extend_parameter(1.75, Extend::Reflect) - 0.25).abs() < 1e-6);
        assert!((extend_parameter(2.25, Extend::Reflect) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn non_finite_inputs_do_not_panic() {
        assert!((extend_parameter(f32::NAN, Extend::Repeat) - 0.0).abs() < 1e-6);
        assert!(linear_parameter((0.0, 0.0), (0.0, 0.0), 1.0, 1.0).is_none());
        assert!(linear_parameter((0.0, 0.0), (f32::NAN, 0.0), 1.0, 1.0).is_none());
        assert!(radial_parameter((0.0, 0.0), f32::NAN, (1.0, 0.0), 1.0, 0.5, 0.0).is_none());
        assert!(sweep_parameter((0.0, 0.0), 0.0, 0.0, 1.0, 1.0).is_none());
    }

    #[test]
    fn linear_gradient_endpoints() {
        let p0 = (0.0, 0.0);
        let p1 = (10.0, 0.0);
        assert!(
            (linear_parameter(p0, p1, 0.0, 5.0).expect("valid gradient geometry") - 0.0).abs()
                < 1e-6
        );
        assert!(
            (linear_parameter(p0, p1, 5.0, 5.0).expect("valid gradient geometry") - 0.5).abs()
                < 1e-6
        );
        assert!(
            (linear_parameter(p0, p1, 10.0, 5.0).expect("valid gradient geometry") - 1.0).abs()
                < 1e-6
        );
    }

    #[test]
    fn solid_brush_alpha_multiplier() {
        let c = solid_color(0, 0.5, resolve);
        assert!((c.a - 0.5).abs() < 1e-6);
        let c = solid_color(0, f32::INFINITY, resolve);
        assert_eq!(c.a, 0.0);
    }

    #[test]
    fn empty_and_single_stops() {
        let color = sample_stops(&[], 0.5, &resolve);
        assert_eq!(color.a, 0.0);
        let stops = [ColorStop {
            offset: 0.0,
            palette_index: 0,
            alpha: 1.0,
        }];
        let color = sample_stops(&stops, 0.9, &resolve);
        assert!((color.r - 1.0).abs() < 1e-6);
    }

    #[test]
    fn radial_basic_circle() {
        // Unit circle centered at origin: point at radius 0.5 -> t = 0.5
        let t = radial_parameter((0.0, 0.0), 0.0, (0.0, 0.0), 1.0, 0.5, 0.0)
            .expect("valid gradient geometry");
        assert!((t - 0.5).abs() < 1e-5);
    }

    #[test]
    fn sweep_basic_angles() {
        // Start 0, end 360, point at +x axis is angle 0.
        let t = sweep_parameter((0.0, 0.0), 0.0, 360.0, 1.0, 0.0).expect("valid gradient geometry");
        assert!(t.abs() < 1e-5);
        // Point at -y (clockwise 90 degrees).
        let t =
            sweep_parameter((0.0, 0.0), 0.0, 360.0, 0.0, -1.0).expect("valid gradient geometry");
        assert!((t - 0.25).abs() < 1e-5);
    }
}
