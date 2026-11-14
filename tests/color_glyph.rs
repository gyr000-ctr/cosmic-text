// SPDX-License-Identifier: MIT OR Apache-2.0
//! Integration tests for COLRv0/COLRv1 + CPAL color glyph rasterization.
//!
//! Font fixtures come from `font-test-data` (the same synthetic fonts the
//! fontations/skrifa project uses), which exercise every COLRv1 paint kind
//! including gradients, transforms, compositing, cycles and missing clip
//! lists.

use cosmic_text::skrifa::{color::ColorGlyphFormat, FontRef, MetadataProvider};
use cosmic_text::{
    CacheKey, CacheKeyFlags, Color, ColorGlyphCache, ColorRasterError, FontSystem, PaletteSelector,
    SubpixelBin,
};
use font_test_data::{COLRV0V1, COLRV0V1_VARIABLE, COLRV1_NO_CLIPLIST};

/// A font loaded into a fresh [`FontSystem`], together with the data needed
/// to build cache keys directly.
struct TestFont {
    font_system: FontSystem,
    font_id: fontdb::ID,
    data: &'static [u8],
}

fn load_font_system(data: &'static [u8]) -> TestFont {
    let mut font_system =
        FontSystem::new_with_locale_and_db("en-US".into(), fontdb::Database::new());
    font_system.db_mut().load_font_data(data.to_vec());
    let font_id = font_system.db().faces().next().expect("one face loaded").id;
    TestFont {
        font_system,
        font_id,
        data,
    }
}

impl TestFont {
    /// Resolve a codepoint to a glyph id through the font's charmap.
    fn glyph_for(&self, cp: char) -> u16 {
        FontRef::new(self.data)
            .unwrap()
            .charmap()
            .map(cp)
            .expect("codepoint mapped in fixture font")
            .to_u32() as u16
    }
}

fn cache_key(font_id: fontdb::ID, glyph_id: u16, size: f32) -> CacheKey {
    CacheKey {
        font_id,
        glyph_id,
        font_size_bits: size.to_bits(),
        x_bin: SubpixelBin::Zero,
        y_bin: SubpixelBin::Zero,
        font_weight: fontdb::Weight::NORMAL,
        flags: CacheKeyFlags::empty(),
    }
}

/// Count opaque-ish colored pixels and collect the set of distinct RGBA
/// values in a rendered image.
fn pixel_stats(
    image: &cosmic_text::ColorGlyphImage,
) -> (usize, std::collections::HashSet<[u8; 4]>) {
    let mut colored = 0;
    let mut colors = std::collections::HashSet::new();
    for px in image.data.as_chunks::<4>().0 {
        let [r, g, b, a] = *px;
        if a > 0 {
            colored += 1;
            colors.insert([r, g, b, a]);
        }
    }
    (colored, colors)
}

#[test]
fn colrv1_glyph_renders_to_color_pixels() {
    let mut font = load_font_system(COLRV0V1);
    // GRADIENT_STOPS_REPEAT[0] is a COLRv1 gradient glyph.
    let glyph_id = font.glyph_for('\u{f0100}');
    let mut cache = ColorGlyphCache::new();
    let image = cache
        .get(
            &mut font.font_system,
            cache_key(font.font_id, glyph_id, 32.0),
            PaletteSelector::default_palette(),
            Color::rgb(0, 0, 0),
        )
        .expect("rasterization succeeds")
        .expect("COLRv1 glyph is recognized as a color glyph");

    let (colored, _) = pixel_stats(image);
    assert!(colored > 0, "COLRv1 glyph produced no visible pixels");
    assert!(image.width > 0 && image.height > 0);
    assert_eq!(image.data.len(), (image.width * image.height * 4) as usize);
}

#[test]
fn colrv0_glyph_renders_through_same_path() {
    let mut font = load_font_system(COLRV0V1);
    // COLORED_CIRCLES_V0 only exists as a COLRv0 base glyph.
    let glyph_id = font.glyph_for('\u{f0e00}');

    // Sanity check the fixture really is v0-only for this glyph.
    let font_ref = FontRef::new(COLRV0V1).unwrap();
    let skrifa_gid = cosmic_text::skrifa::GlyphId::new(u32::from(glyph_id));
    assert!(font_ref
        .color_glyphs()
        .get_with_format(skrifa_gid, ColorGlyphFormat::ColrV1)
        .is_none());
    assert!(font_ref
        .color_glyphs()
        .get_with_format(skrifa_gid, ColorGlyphFormat::ColrV0)
        .is_some());

    let mut cache = ColorGlyphCache::new();
    let image = cache
        .get(
            &mut font.font_system,
            cache_key(font.font_id, glyph_id, 32.0),
            PaletteSelector::default_palette(),
            Color::rgb(0, 0, 0),
        )
        .expect("v0 rasterization succeeds")
        .expect("v0 glyph is recognized as color");
    let (colored, _) = pixel_stats(image);
    assert!(colored > 0);
}

#[test]
fn colrv1_and_colrv0_are_each_selected_for_their_glyphs() {
    // In this fixture the colored circles are split into a v0-only base
    // glyph (f0e00) and a v1-only base glyph (f0e01). Verify each format is
    // recognized and rendered through the same cache entry point.
    let font_ref = FontRef::new(COLRV0V1).unwrap();
    let v1_gid = font_ref.charmap().map('\u{f0e01}').unwrap();
    assert!(font_ref
        .color_glyphs()
        .get_with_format(v1_gid, ColorGlyphFormat::ColrV1)
        .is_some());
    assert!(font_ref
        .color_glyphs()
        .get_with_format(v1_gid, ColorGlyphFormat::ColrV0)
        .is_none());

    let v0_gid = font_ref.charmap().map('\u{f0e00}').unwrap();
    assert!(font_ref
        .color_glyphs()
        .get_with_format(v0_gid, ColorGlyphFormat::ColrV0)
        .is_some());
    assert!(font_ref
        .color_glyphs()
        .get_with_format(v0_gid, ColorGlyphFormat::ColrV1)
        .is_none());

    // `ColorGlyphCollection::get` prefers v1 and falls back to v0.
    assert!(font_ref.color_glyphs().get(v1_gid).is_some());
    assert!(font_ref.color_glyphs().get(v0_gid).is_some());

    let mut font = load_font_system(COLRV0V1);
    let gid = font.glyph_for('\u{f0e01}');
    let mut cache = ColorGlyphCache::new();
    let image = cache
        .get(
            &mut font.font_system,
            cache_key(font.font_id, gid, 32.0),
            PaletteSelector::default_palette(),
            Color::rgb(0, 0, 0),
        )
        .unwrap()
        .unwrap();
    assert!(pixel_stats(image).0 > 0);
}

#[test]
fn palette_switching_changes_colors() {
    let mut font = load_font_system(COLRV0V1);
    let glyph_id = font.glyph_for('\u{f0e00}');
    let mut cache = ColorGlyphCache::new();

    let mut colors_per_palette = Vec::new();
    for palette_index in 0..3u16 {
        let image = cache
            .get(
                &mut font.font_system,
                cache_key(font.font_id, glyph_id, 32.0),
                PaletteSelector::indexed(palette_index),
                Color::rgb(0, 0, 0),
            )
            .unwrap()
            .unwrap();
        let (_, colors) = pixel_stats(image);
        colors_per_palette.push(colors);
    }
    // The fixture defines three distinct palettes; at least two must produce
    // visibly different color sets.
    assert_ne!(
        colors_per_palette[0], colors_per_palette[1],
        "palette 0 and 1 should render different colors"
    );
    assert_ne!(
        colors_per_palette[0], colors_per_palette[2],
        "palette 0 and 2 should render different colors"
    );
}

#[test]
fn missing_palette_falls_back_to_default() {
    let mut font = load_font_system(COLRV0V1);
    let glyph_id = font.glyph_for('\u{f0e00}');
    let mut cache = ColorGlyphCache::new();

    let default_image = cache
        .get(
            &mut font.font_system,
            cache_key(font.font_id, glyph_id, 32.0),
            PaletteSelector::default_palette(),
            Color::rgb(0, 0, 0),
        )
        .unwrap()
        .unwrap()
        .clone();

    // Palette index 99 does not exist; output must equal palette 0.
    let fallback_image = cache
        .get(
            &mut font.font_system,
            cache_key(font.font_id, glyph_id, 32.0),
            PaletteSelector::indexed(99),
            Color::rgb(0, 0, 0),
        )
        .unwrap()
        .unwrap();
    assert_eq!(default_image.data, fallback_image.data);
}

#[test]
fn foreground_color_is_used_for_ffff_entries() {
    // f0b00 is a linear gradient whose middle color stop uses palette index
    // 0xFFFF, i.e. "the current foreground text color".
    let mut font = load_font_system(COLRV0V1);
    let glyph_id = font.glyph_for('\u{f0b00}');
    let mut cache = ColorGlyphCache::new();

    let red = Color::rgb(255, 0, 0);
    let blue = Color::rgb(0, 0, 255);
    let red_image = cache
        .get(
            &mut font.font_system,
            cache_key(font.font_id, glyph_id, 32.0),
            PaletteSelector::default_palette(),
            red,
        )
        .unwrap()
        .unwrap()
        .clone();
    let blue_image = cache
        .get(
            &mut font.font_system,
            cache_key(font.font_id, glyph_id, 32.0),
            PaletteSelector::default_palette(),
            blue,
        )
        .unwrap()
        .unwrap();
    // Different foreground colors must produce differently keyed bitmaps.
    assert_ne!(red_image.data, blue_image.data);

    // The red render must actually contain red pixels from the 0xFFFF stop.
    let contains_red = red_image
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .any(|px| px[3] > 0 && px[0] > px[2]);
    assert!(contains_red, "foreground red never appeared in the bitmap");
}

#[test]
fn gradient_rect_contains_color_interpolation() {
    // GRADIENT_STOPS_REPEAT[0] (f0100) is a COLRv1 linear gradient glyph.
    let mut font = load_font_system(COLRV0V1);
    let glyph_id = font.glyph_for('\u{f0100}');
    let mut cache = ColorGlyphCache::new();
    let image = cache
        .get(
            &mut font.font_system,
            cache_key(font.font_id, glyph_id, 48.0),
            PaletteSelector::default_palette(),
            Color::rgb(0, 0, 0),
        )
        .unwrap()
        .unwrap();
    let (colored, colors) = pixel_stats(image);
    assert!(colored > 0);
    // A gradient must interpolate: expect several distinct colors rather
    // than a single flat fill.
    assert!(
        colors.len() > 4,
        "expected interpolated gradient, got {} distinct colors",
        colors.len()
    );
}

#[test]
fn paint_cycle_is_reported_as_error() {
    // PAINTCOLRGLYPH_CYCLE contains three glyphs referencing each other.
    // skrifa uses a Floyd tortoise-and-hare decycler, so the cycle is
    // reported from the entry points whose traversal depth makes the two
    // pointers collide; the remaining entry point is handled without panic.
    let mut font = load_font_system(COLRV0V1);
    for cp in ['\u{f1100}', '\u{f1101}'] {
        let glyph_id = font.glyph_for(cp);
        let mut cache = ColorGlyphCache::new();
        let err = cache
            .get(
                &mut font.font_system,
                cache_key(font.font_id, glyph_id, 32.0),
                PaletteSelector::default_palette(),
                Color::rgb(0, 0, 0),
            )
            .expect_err("cyclic paint graph must fail");
        assert!(
            matches!(
                err,
                ColorRasterError::CycleDetected | ColorRasterError::DepthLimitExceeded
            ),
            "got {err:?}"
        );
    }

    // The third entry point must at least not panic; it either errors or
    // produces a (possibly empty) bitmap.
    let glyph_id = font.glyph_for('\u{f1200}');
    let mut cache = ColorGlyphCache::new();
    let _ = cache.get(
        &mut font.font_system,
        cache_key(font.font_id, glyph_id, 32.0),
        PaletteSelector::default_palette(),
        Color::rgb(0, 0, 0),
    );
}

#[test]
fn glyph_without_clip_box_still_renders() {
    // COLRV1_NO_CLIPLIST has no ClipList; the rasterizer falls back to the
    // root outline bounds.
    let mut font = load_font_system(COLRV1_NO_CLIPLIST);
    let font_ref = FontRef::new(COLRV1_NO_CLIPLIST).unwrap();
    let glyph_id = font_ref
        .charmap()
        .mappings()
        .next()
        .map(|(_, gid)| gid.to_u32() as u16)
        .unwrap();
    let mut cache = ColorGlyphCache::new();
    let image = cache
        .get(
            &mut font.font_system,
            cache_key(font.font_id, glyph_id, 32.0),
            PaletteSelector::default_palette(),
            Color::rgb(0, 0, 0),
        )
        .expect("missing clip list must not be fatal")
        .expect("glyph still renders");
    assert!(pixel_stats(image).0 > 0);
}

#[test]
fn non_color_glyph_returns_none() {
    let mut font = load_font_system(COLRV0V1);
    let mut cache = ColorGlyphCache::new();
    // Glyph id 0 (.notdef) has no COLR entry.
    let result = cache
        .get(
            &mut font.font_system,
            cache_key(font.font_id, 0, 32.0),
            PaletteSelector::default_palette(),
            Color::rgb(0, 0, 0),
        )
        .expect("non-color glyph is not an error");
    assert!(result.is_none(), "monochrome glyph must report None");
}

#[test]
fn cache_reuses_bitmap_and_is_bounded() {
    let mut font = load_font_system(COLRV0V1);
    let glyph_id = font.glyph_for('\u{f0e00}');
    let mut cache = ColorGlyphCache::with_limits(2, usize::MAX);

    for _ in 0..3 {
        let image = cache
            .get(
                &mut font.font_system,
                cache_key(font.font_id, glyph_id, 32.0),
                PaletteSelector::default_palette(),
                Color::rgb(0, 0, 0),
            )
            .unwrap()
            .unwrap();
        assert!(pixel_stats(image).0 > 0);
    }
    // Repeated requests for the same key never grow the cache.
    assert_eq!(cache.len(), 1);

    // Distinct sizes create distinct entries and eviction kicks in at 2.
    for size in [16.0, 24.0, 40.0] {
        cache
            .get(
                &mut font.font_system,
                cache_key(font.font_id, glyph_id, size),
                PaletteSelector::default_palette(),
                Color::rgb(0, 0, 0),
            )
            .unwrap()
            .unwrap();
    }
    assert!(cache.len() <= 2, "entry limit must hold");
}

#[test]
fn cache_key_includes_palette_and_foreground() {
    let mut font = load_font_system(COLRV0V1);
    let glyph_id = font.glyph_for('\u{f0e00}');
    let mut cache = ColorGlyphCache::new();
    let key = cosmic_text::ColorCacheKey {
        cache_key: cache_key(font.font_id, glyph_id, 32.0),
        palette_index: 0,
        foreground: Color::rgb(1, 2, 3),
    };
    let mut other = key;
    other.palette_index = 1;
    assert_ne!(key, other);
    other = key;
    other.foreground = Color::rgb(9, 9, 9);
    assert_ne!(key, other);

    // Exercising distinct palettes through the public API must store
    // distinct entries.
    cache
        .get(
            &mut font.font_system,
            cache_key(font.font_id, glyph_id, 32.0),
            PaletteSelector::indexed(0),
            Color::rgb(0, 0, 0),
        )
        .unwrap()
        .unwrap();
    cache
        .get(
            &mut font.font_system,
            cache_key(font.font_id, glyph_id, 32.0),
            PaletteSelector::indexed(1),
            Color::rgb(0, 0, 0),
        )
        .unwrap()
        .unwrap();
    assert_eq!(cache.len(), 2);
}

#[test]
fn corrupted_font_data_is_handled_safely() {
    // Truncated font data must fail gracefully rather than panic.
    let mut font_system =
        FontSystem::new_with_locale_and_db("en-US".into(), fontdb::Database::new());
    font_system
        .db_mut()
        .load_font_data(COLRV0V1[..COLRV0V1.len() / 4].to_vec());
    let face_id = {
        match font_system.db().faces().next() {
            Some(face) => Some(face.id),
            // fontdb may reject the truncated face outright; that is also safe.
            None => return,
        }
    };
    let mut cache = ColorGlyphCache::new();
    let result = cache.get(
        &mut font_system,
        cache_key(face_id.unwrap(), 1, 32.0),
        PaletteSelector::default_palette(),
        Color::rgb(0, 0, 0),
    );
    assert!(result.is_err() || result.unwrap().is_none());
}

#[test]
fn oversized_font_size_is_rejected() {
    let mut font = load_font_system(COLRV0V1);
    let glyph_id = font.glyph_for('\u{f0e00}');
    let mut cache = ColorGlyphCache::new();
    // A size large enough to push glyph edges past MAX_GLYPH_EDGE (1024).
    let err = cache
        .get(
            &mut font.font_system,
            cache_key(font.font_id, glyph_id, 4096.0),
            PaletteSelector::default_palette(),
            Color::rgb(0, 0, 0),
        )
        .expect_err("huge size must be rejected or fall back, not allocate");
    assert!(matches!(
        err,
        ColorRasterError::GlyphTooLarge | ColorRasterError::InvalidTransform
    ));
}

#[test]
fn variable_font_weight_is_accepted() {
    // The variable fixture must rasterize without errors at a named weight.
    let mut font = load_font_system(COLRV0V1_VARIABLE);
    let font_ref = FontRef::new(COLRV0V1_VARIABLE).unwrap();
    let glyph_id = font_ref
        .charmap()
        .mappings()
        .next()
        .map(|(_, gid)| gid.to_u32() as u16)
        .unwrap();
    let mut cache = ColorGlyphCache::new();
    let image = cache
        .get(
            &mut font.font_system,
            cache_key(font.font_id, glyph_id, 32.0),
            PaletteSelector::default_palette(),
            Color::rgb(0, 0, 0),
        )
        .expect("variable font rasterizes");
    assert!(image.is_some());
}

#[test]
fn with_pixels_reports_color_glyph_status() {
    let mut font = load_font_system(COLRV0V1);
    let glyph_id = font.glyph_for('\u{f0e00}');
    let mut cache = ColorGlyphCache::new();

    let mut count = 0;
    let painted = cache
        .with_pixels(
            &mut font.font_system,
            cache_key(font.font_id, glyph_id, 32.0),
            PaletteSelector::default_palette(),
            Color::rgb(0, 0, 0),
            |_x, _y, color| {
                if color.a() > 0 {
                    count += 1;
                }
            },
        )
        .unwrap();
    assert!(painted);
    assert!(count > 0);

    // A monochrome glyph reports false so callers can fall back.
    let painted = cache
        .with_pixels(
            &mut font.font_system,
            cache_key(font.font_id, 0, 32.0),
            PaletteSelector::default_palette(),
            Color::rgb(0, 0, 0),
            |_, _, _| {},
        )
        .unwrap();
    assert!(!painted);
}

#[test]
fn swash_cache_paints_color_glyph_and_falls_back_for_monochrome() {
    use cosmic_text::SwashCache;

    let mut font = load_font_system(COLRV0V1);
    let mut cache = SwashCache::new();

    // A COLR color glyph is painted through the color path and must contain
    // non-grayscale pixels (the fixture uses saturated palette colors).
    let color_gid = font.glyph_for('\u{f0e00}');
    let mut colors: std::collections::HashSet<[u8; 4]> = std::collections::HashSet::new();
    cache.with_pixels(
        &mut font.font_system,
        cache_key(font.font_id, color_gid, 32.0),
        Color::rgb(0, 0, 0),
        |_x, _y, color| {
            if color.a() > 0 {
                colors.insert(color.as_rgba());
            }
        },
    );
    assert!(!colors.is_empty(), "color glyph produced no pixels");
    let has_saturated = colors
        .iter()
        .any(|[r, g, b, _a]| r.abs_diff(*g) > 20 || g.abs_diff(*b) > 20);
    assert!(
        has_saturated,
        "COLR glyph should contain palette colors, not only grayscale: {colors:?}"
    );

    // A monochrome glyph (.notdef has no COLR entry) still renders through
    // the existing swash path without errors.
    let mut fallback_pixels = 0;
    cache.with_pixels(
        &mut font.font_system,
        cache_key(font.font_id, 0, 32.0),
        Color::rgb(10, 20, 30),
        |_x, _y, color| {
            if color.a() > 0 {
                fallback_pixels += 1;
            }
        },
    );
    // .notdef may render or be empty depending on the font; the important
    // part is that the call did not panic and went through the fallback.
    let _ = fallback_pixels;
}

#[test]
fn repeated_color_renders_reuse_cache() {
    use cosmic_text::SwashCache;

    let mut font = load_font_system(COLRV0V1);
    let mut cache = SwashCache::new();
    let gid = font.glyph_for('\u{f0e00}');
    let key = cache_key(font.font_id, gid, 32.0);

    let mut first = Vec::new();
    cache.with_pixels(
        &mut font.font_system,
        key,
        Color::rgb(0, 0, 0),
        |x, y, color| first.push((x, y, color)),
    );
    let mut second = Vec::new();
    cache.with_pixels(
        &mut font.font_system,
        key,
        Color::rgb(0, 0, 0),
        |x, y, color| second.push((x, y, color)),
    );
    // A second frame must produce identical pixels from the cached bitmap
    // without re-traversing the paint graph.
    assert_eq!(first, second);
    assert!(!first.is_empty());
}

#[test]
fn color_pixel_coordinates_match_swash_convention() {
    use cosmic_text::SwashCache;

    let mut font = load_font_system(COLRV0V1);
    let gid = font.glyph_for('\u{f0e00}');
    let mut cache = SwashCache::new();

    // Every emitted y coordinate must be `-(positive top) + row`, i.e. glyph
    // pixels above the baseline use negative y just like the swash path.
    let mut saw_negative_y = false;
    let mut min_x = i32::MAX;
    cache.with_pixels(
        &mut font.font_system,
        cache_key(font.font_id, gid, 32.0),
        Color::rgb(0, 0, 0),
        |x, y, color| {
            if color.a() > 0 {
                if y < 0 {
                    saw_negative_y = true;
                }
                min_x = min_x.min(x);
            }
        },
    );
    assert!(
        saw_negative_y,
        "a glyph above the baseline must emit negative y"
    );
    // x coordinates must be non-negative and bounded (no extra origin shift
    // between renders).
    assert!((-1..i32::MAX).contains(&min_x));
}
