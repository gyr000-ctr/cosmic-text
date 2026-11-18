use common::DrawTestCfg;
use cosmic_text::Attrs;
use fontdb::Family;

mod common;

#[test]
fn test_hebrew_word_rendering() {
    let attrs = Attrs::new().family(Family::Name("Noto Sans"));
    DrawTestCfg::new("a_hebrew_word")
        .font_size(36., 40.)
        .font_attrs(attrs)
        .text("בדיקה")
        .canvas(120, 60)
        .validate_text_rendering();
}

#[test]
fn test_hebrew_paragraph_rendering() {
    let paragraph = "השועל החום המהיר קופץ מעל הכלב העצלן";
    let attrs = Attrs::new().family(Family::Name("Noto Sans"));
    DrawTestCfg::new("a_hebrew_paragraph")
        .font_size(36., 40.)
        .font_attrs(attrs)
        .text(paragraph)
        .canvas(400, 110)
        .validate_text_rendering();
}

#[test]
fn test_english_mixed_with_hebrew_paragraph_rendering() {
    let paragraph = "Many computer programs fail to display bidirectional text correctly. For example, this page is mostly LTR English script, and here is the RTL Hebrew name Sarah: שרה, spelled sin (ש) on the right, resh (ר) in the middle, and heh (ה) on the left.";
    let attrs = Attrs::new().family(Family::Name("Noto Sans"));
    DrawTestCfg::new("some_english_mixed_with_hebrew")
        .font_size(16., 20.)
        .font_attrs(attrs)
        .text(paragraph)
        .canvas(400, 120)
        .validate_text_rendering();
}

#[test]
fn test_arabic_word_rendering() {
    let attrs = Attrs::new().family(Family::Name("Noto Sans"));
    DrawTestCfg::new("an_arabic_word")
        .font_size(36., 40.)
        .font_attrs(attrs)
        .text("خالصة")
        .canvas(120, 60)
        .validate_text_rendering();
}

#[test]
fn test_arabic_paragraph_rendering() {
    let paragraph = "الثعلب البني السريع يقفز فوق الكلب الكسول";
    let attrs = Attrs::new().family(Family::Name("Noto Sans"));
    DrawTestCfg::new("an_arabic_paragraph")
        .font_size(36., 40.)
        .font_attrs(attrs)
        .text(paragraph)
        .canvas(400, 110)
        .validate_text_rendering();
}

#[test]
fn test_english_mixed_with_arabic_paragraph_rendering() {
    let paragraph = "I like to render اللغة العربية in Rust!";
    let attrs = Attrs::new().family(Family::Name("Noto Sans"));
    DrawTestCfg::new("some_english_mixed_with_arabic")
        .font_size(36., 40.)
        .font_attrs(attrs)
        .text(paragraph)
        .canvas(400, 110)
        .validate_text_rendering();
}

#[test]
fn test_ligature_segmentation() {
    use cosmic_text::{Buffer, FontSystem, Metrics, Shaping};

    let mut font_system =
        FontSystem::new_with_locale_and_db("en-US".into(), fontdb::Database::new());
    let font = std::fs::read("fonts/Inter-Regular.ttf").unwrap();
    font_system.db_mut().load_font_data(font);
    let metrics = Metrics::new(14.0, 20.0);

    let mut buffer = Buffer::new(&mut font_system, metrics);
    let mut buffer = buffer.borrow_with(&mut font_system);

    buffer.set_text("|>", &Attrs::new(), Shaping::Advanced, None);
    let _ = buffer.layout_runs();

    let line = &buffer.lines[0];
    let shape = line.shape_opt().expect("ShapeLine not found");
    let span = &shape.spans[0];

    // Inter-Regular HAS a contextual alternate for |> (changing the glyph ID),
    // so our probe detects it and keeps them together.
    assert_eq!(
        span.words.len(),
        1,
        "Expected '|>' to be 1 word (contextual alternate in Inter), but found {} words.",
        span.words.len()
    );

    // Test -> (Arrow), which is a common ligature.
    buffer.set_text("->", &Attrs::new(), Shaping::Advanced, None);
    let _ = buffer.layout_runs();
    let line = &buffer.lines[0];
    let shape = line.shape_opt().expect("ShapeLine not found");

    assert_eq!(
        shape.spans[0].words.len(),
        1,
        "Expected '->' to be a single word (ligature), but found {} words.",
        shape.spans[0].words.len()
    );

    // Test !=
    buffer.set_text("!=", &Attrs::new(), Shaping::Advanced, None);
    let _ = buffer.layout_runs();
    let line = &buffer.lines[0];
    let shape = line.shape_opt().expect("ShapeLine not found");
    // Inter has a contextual alternate for != too.
    assert_eq!(
        shape.spans[0].words.len(),
        1,
        "Expected '!=' to be 1 word (contextual alternate), but found {} words.",
        shape.spans[0].words.len()
    );

    // Test ++
    buffer.set_text("++", &Attrs::new(), Shaping::Advanced, None);
    let _ = buffer.layout_runs();
    let line = &buffer.lines[0];
    let shape = line.shape_opt().expect("ShapeLine not found");
    // Inter does not have a ++ ligature.
    assert_eq!(
        shape.spans[0].words.len(),
        2,
        "Expected '++' to be 2 words, but found {} words.",
        shape.spans[0].words.len()
    );
}

#[test]
fn test_mixed_direction_paragraphs_do_not_panic() {
    use cosmic_text::{AttrsList, Direction, FontSystem, ShapeLine, Shaping};

    let mut font_system =
        FontSystem::new_with_locale_and_db("en-US".into(), fontdb::Database::new());
    font_system
        .db_mut()
        .load_font_data(std::fs::read("fonts/Inter-Regular.ttf").unwrap());
    font_system
        .db_mut()
        .load_font_data(std::fs::read("fonts/NotoSansHebrew.ttf").unwrap());

    // Latin 'A' and Hebrew Aleph separated by each BidiClass::B code point, including
    // PS (U+2029) and the ASCII FS (U+001C), to cover more than just newlines.
    for sep in [
        '\u{000A}', '\u{000D}', '\u{001C}', '\u{001D}', '\u{001E}', '\u{0085}', '\u{2029}',
    ] {
        let line = format!("A{sep}\u{05D0}");
        ShapeLine::new(
            &mut font_system,
            &line,
            &AttrsList::new(&Attrs::new()),
            Shaping::Advanced,
            8,
            Direction::Auto,
        );
    }
}

mod synthetic_bold {
    use cosmic_text::{
        fontdb, Attrs, Buffer, CacheKey, CacheKeyFlags, Family, FontSystem, Metrics, Shaping,
        SwashCache, SwashImage, SyntheticBold, SyntheticBoldError, Weight,
    };

    fn font_system_with_inter_regular() -> FontSystem {
        let mut font_system =
            FontSystem::new_with_locale_and_db("en-US".into(), fontdb::Database::new());
        font_system
            .db_mut()
            .load_font_data(std::fs::read("fonts/Inter-Regular.ttf").unwrap());
        font_system
    }

    fn inter_family_name(font_system: &FontSystem) -> String {
        font_system.db().faces().next().unwrap().families[0]
            .0
            .clone()
    }

    fn shape_flags(
        font_system: &mut FontSystem,
        attrs: &Attrs,
        text: &str,
    ) -> Vec<CacheKeyFlags> {
        let metrics = Metrics::new(16.0, 20.0);
        let mut buffer = Buffer::new(font_system, metrics);
        let mut buffer = buffer.borrow_with(font_system);
        buffer.set_size(Some(300.0), Some(100.0));
        buffer.set_text(text, attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(true);
        buffer
            .layout_runs()
            .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.cache_key_flags))
            .collect()
    }

    fn shape_first_cache_key(
        font_system: &mut FontSystem,
        attrs: &Attrs,
        text: &str,
    ) -> CacheKey {
        let metrics = Metrics::new(32.0, 40.0);
        let mut buffer = Buffer::new(font_system, metrics);
        let mut buffer = buffer.borrow_with(font_system);
        buffer.set_size(Some(300.0), Some(100.0));
        buffer.set_text(text, attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(true);
        let glyph = buffer.layout_runs().next().unwrap().glyphs[0].clone();
        glyph.physical((0.0, 0.0), 1.0).cache_key
    }

    fn ink(image: &SwashImage) -> u64 {
        image.data.iter().map(|&byte| u64::from(byte)).sum()
    }

    #[test]
    fn strength_out_of_range_is_a_distinct_error() {
        for bad in [
            0.0,
            -0.01,
            SyntheticBold::MAX_STRENGTH + 0.01,
            f32::NAN,
            f32::INFINITY,
        ] {
            assert!(
                matches!(
                    SyntheticBold::with_strength(bad),
                    Err(SyntheticBoldError::StrengthOutOfRange { .. })
                ),
                "strength {bad} must be rejected"
            );
        }
        assert!(SyntheticBold::with_strength(SyntheticBold::MAX_STRENGTH).is_ok());
        assert!(SyntheticBold::with_strength(0.001).is_ok());

        // The two error kinds are distinguishable from each other
        let strength_err = SyntheticBold::with_strength(1.0).unwrap_err();
        assert!(!matches!(
            strength_err,
            SyntheticBoldError::EmptyGlyph { .. }
        ));
    }

    #[test]
    fn strength_is_packed_into_cache_key_flags() {
        let weak = CacheKeyFlags::empty().with_synthetic_bold(0.02);
        let strong = CacheKeyFlags::empty().with_synthetic_bold(0.1);

        assert!(weak.contains(CacheKeyFlags::FAKE_BOLD));
        assert!(strong.contains(CacheKeyFlags::FAKE_BOLD));
        // Different strengths produce different cache keys
        assert_ne!(weak, strong);
        // And round-trip with quantization error well below a visible step
        assert!((weak.synthetic_bold_strength().unwrap() - 0.02).abs() < 0.001);
        assert!((strong.synthetic_bold_strength().unwrap() - 0.1).abs() < 0.001);
        // No strength without the flag
        assert_eq!(CacheKeyFlags::empty().synthetic_bold_strength(), None);
    }

    #[test]
    fn applies_only_when_real_weight_is_missing() {
        let mut font_system = font_system_with_inter_regular();
        let family = inter_family_name(&font_system);

        // Inter-Regular is a static weight-400 font: requesting 700 with
        // synthetic bold enabled must embolden every glyph.
        let bold_attrs = Attrs::new()
            .family(Family::Name(&family))
            .weight(Weight(700))
            .synthetic_bold(SyntheticBold::new());
        let flags = shape_flags(&mut font_system, &bold_attrs, "Hello");
        assert!(!flags.is_empty());
        assert!(
            flags
                .iter()
                .all(|f| f.contains(CacheKeyFlags::FAKE_BOLD)),
            "expected FAKE_BOLD on all glyphs, got {flags:?}"
        );

        // Requesting the weight the font actually provides must not synthesize.
        let real_attrs = Attrs::new()
            .family(Family::Name(&family))
            .weight(Weight(400))
            .synthetic_bold(SyntheticBold::new());
        let flags = shape_flags(&mut font_system, &real_attrs, "Hello");
        assert!(!flags.is_empty());
        assert!(flags.iter().all(|f| !f.contains(CacheKeyFlags::FAKE_BOLD)));

        // Without the synthetic bold attribute nothing is synthesized either.
        let plain_attrs = Attrs::new()
            .family(Family::Name(&family))
            .weight(Weight(700));
        let flags = shape_flags(&mut font_system, &plain_attrs, "Hello");
        assert!(!flags.is_empty());
        assert!(flags.iter().all(|f| !f.contains(CacheKeyFlags::FAKE_BOLD)));
    }

    #[test]
    fn does_not_change_layout_metrics() {
        let mut font_system = font_system_with_inter_regular();
        let family = inter_family_name(&font_system);

        let layout = |font_system: &mut FontSystem, attrs: &Attrs| {
            let metrics = Metrics::new(16.0, 20.0);
            let mut buffer = Buffer::new(font_system, metrics);
            let mut buffer = buffer.borrow_with(font_system);
            buffer.set_size(Some(120.0), Some(200.0));
            buffer.set_text("Hello world, synthetic bold", attrs, Shaping::Advanced, None);
            buffer.shape_until_scroll(true);
            buffer
                .layout_runs()
                .map(|run| {
                    (
                        run.line_y,
                        run.line_top,
                        run.line_height,
                        run.line_w,
                        run.glyphs.iter().map(|g| g.w).collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>()
        };

        let plain = layout(
            &mut font_system,
            &Attrs::new()
                .family(Family::Name(&family))
                .weight(Weight(700)),
        );
        let bold = layout(
            &mut font_system,
            &Attrs::new()
                .family(Family::Name(&family))
                .weight(Weight(700))
                .synthetic_bold(SyntheticBold::new()),
        );

        assert!(!plain.is_empty());
        assert_eq!(
            plain, bold,
            "synthetic bold must not change advances or line metrics"
        );
    }

    #[test]
    fn renders_bolder_and_cache_distinguishes_synthetic_from_real() {
        let mut font_system = font_system_with_inter_regular();
        let family = inter_family_name(&font_system);
        let attrs = Attrs::new()
            .family(Family::Name(&family))
            .weight(Weight(700))
            .synthetic_bold(SyntheticBold::new());

        let bold_key = shape_first_cache_key(&mut font_system, &attrs, "H");
        assert!(bold_key.flags.contains(CacheKeyFlags::FAKE_BOLD));

        let mut real_key = bold_key;
        real_key.flags = CacheKeyFlags::empty();

        let mut swash_cache = SwashCache::new();
        let real_ink = ink(
            swash_cache
                .get_image(&mut font_system, real_key)
                .as_ref()
                .unwrap(),
        );
        let bold_ink = ink(
            swash_cache
                .try_get_image(&mut font_system, bold_key)
                .unwrap()
                .as_ref()
                .unwrap(),
        );

        assert!(
            bold_ink > real_ink,
            "synthetic bold must add ink (bold {bold_ink} <= real {real_ink})"
        );
        // Synthetic and real renderings live under distinct cache keys
        assert_eq!(swash_cache.image_cache.len(), 2);
    }

    #[test]
    fn empty_glyph_is_a_distinct_error() {
        let mut font_system = font_system_with_inter_regular();
        let family = inter_family_name(&font_system);
        let attrs = Attrs::new()
            .family(Family::Name(&family))
            .weight(Weight(700))
            .synthetic_bold(SyntheticBold::new());

        // A space has no outline pixels to embolden
        let cache_key = shape_first_cache_key(&mut font_system, &attrs, " ");
        assert!(cache_key.flags.contains(CacheKeyFlags::FAKE_BOLD));

        let mut swash_cache = SwashCache::new();
        let err = match swash_cache.try_get_image(&mut font_system, cache_key) {
            Ok(_) => panic!("expected EmptyGlyph error for a space glyph"),
            Err(err) => err,
        };
        assert!(
            matches!(err, SyntheticBoldError::EmptyGlyph { .. }),
            "expected EmptyGlyph, got {err:?}"
        );
        // Distinguishable from the strength error
        assert!(!matches!(
            err,
            SyntheticBoldError::StrengthOutOfRange { .. }
        ));
    }
}
