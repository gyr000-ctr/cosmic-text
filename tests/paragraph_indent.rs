use cosmic_text::{
    fontdb, Align, Attrs, Buffer, Family, FontSystem, Indent, IndentError, Metrics, Shaping, Wrap,
};

const FONT: &str = "Inter";

fn test_font_system() -> FontSystem {
    let mut db = fontdb::Database::new();
    db.load_font_data(std::fs::read("fonts/Inter-Regular.ttf").unwrap());
    FontSystem::new_with_locale_and_db("en-US".into(), db)
}

fn shaped_buffer(font_system: &mut FontSystem, text: &str, width: f32, wrap: Wrap) -> Buffer {
    let mut buffer = Buffer::new(font_system, Metrics::new(16.0, 24.0));
    buffer.set_size(Some(width), Some(1000.0));
    buffer.set_wrap(wrap);
    buffer.set_text(
        text,
        &Attrs::new().family(Family::Name(FONT)),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(font_system, false);
    buffer
}

/// Right edge of the last glyph on a visual line.
fn last_glyph_right(buffer: &Buffer, run_index: usize) -> f32 {
    let run = buffer.layout_runs().nth(run_index).unwrap();
    run.glyphs
        .iter()
        .map(|glyph| glyph.x + glyph.w)
        .fold(f32::NEG_INFINITY, f32::max)
}

fn approx_eq(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

#[test]
fn first_line_indent_only_affects_first_visual_line() {
    let mut font_system = test_font_system();
    let text = "one two three four five six seven eight nine ten";
    let mut buffer = shaped_buffer(&mut font_system, text, 120.0, Wrap::Word);

    // Baseline without indentation.
    let baseline_x: Vec<f32> = buffer
        .layout_runs()
        .map(|run| run.glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min))
        .collect();
    assert!(
        baseline_x.len() >= 3,
        "sample text must soft-wrap several times"
    );

    buffer
        .set_indent(Indent::first_line(20.0))
        .expect("20px indent fits in 120px width");
    buffer.shape_until_scroll(&mut font_system, false);

    let indented_x: Vec<f32> = buffer
        .layout_runs()
        .map(|run| run.glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min))
        .collect();

    // The first visual line moves right by exactly the indent...
    assert!(
        approx_eq(indented_x[0], baseline_x[0] + 20.0),
        "first line should shift by 20px, got {} vs {}",
        indented_x[0],
        baseline_x[0]
    );
    // ...and every soft-wrapped continuation line is untouched.
    for line in 1..baseline_x.len() {
        assert!(
            approx_eq(indented_x[line], baseline_x[line]),
            "continuation line {line} must not be re-indented: {} vs {}",
            indented_x[line],
            baseline_x[line]
        );
    }
}

#[test]
fn hanging_indent_applies_to_continuation_lines_only() {
    let mut font_system = test_font_system();
    let text = "one two three four five six seven eight nine ten";
    let mut buffer = shaped_buffer(&mut font_system, text, 120.0, Wrap::Word);

    let baseline_x: Vec<f32> = buffer
        .layout_runs()
        .map(|run| run.glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min))
        .collect();

    buffer
        .set_indent(Indent::hanging(15.0))
        .expect("15px hanging indent fits");
    buffer.shape_until_scroll(&mut font_system, false);

    let indented_x: Vec<f32> = buffer
        .layout_runs()
        .map(|run| run.glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min))
        .collect();

    assert!(
        approx_eq(indented_x[0], baseline_x[0]),
        "hanging indent must not move the first line: {} vs {}",
        indented_x[0],
        baseline_x[0]
    );
    for line in 1..baseline_x.len() {
        assert!(
            approx_eq(indented_x[line], baseline_x[line] + 15.0),
            "continuation line {line} should shift by 15px: {} vs {}",
            indented_x[line],
            baseline_x[line]
        );
    }
}

#[test]
fn indent_is_per_paragraph_not_per_visual_line() {
    let mut font_system = test_font_system();
    // Two paragraphs, each long enough to soft-wrap.
    let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa\n\
                lorem ipsum dolor sit amet consectetur adipiscing elit sed";
    let mut buffer = shaped_buffer(&mut font_system, text, 100.0, Wrap::Word);

    buffer
        .set_indent(Indent::first_line(18.0))
        .expect("indent fits");
    buffer.shape_until_scroll(&mut font_system, false);

    // Identify the first visual line of each paragraph by line_i.
    let runs: Vec<_> = buffer.layout_runs().collect();
    assert_eq!(
        runs.iter().map(|run| run.line_i).max().unwrap_or(0) + 1,
        2,
        "there are two paragraphs"
    );

    // Every paragraph's first visual line starts at the same indented x.
    let first_x: Vec<f32> = runs
        .iter()
        .enumerate()
        .filter(|(i, run)| *i == 0 || runs[i - 1].line_i != run.line_i)
        .map(|(_, run)| run.glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min))
        .collect();
    assert_eq!(first_x.len(), 2);
    assert!(approx_eq(first_x[0], 18.0));
    assert!(approx_eq(first_x[1], 18.0));

    // Continuation lines keep the unindented left margin (0.0).
    for (i, run) in runs.iter().enumerate() {
        let is_paragraph_start = i == 0 || runs[i - 1].line_i != run.line_i;
        if !is_paragraph_start {
            let x = run.glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min);
            assert!(
                approx_eq(x, 0.0),
                "soft-wrapped line must not repeat indent, got {x}"
            );
        }
    }
}

#[test]
fn indent_changes_wrapping_to_keep_content_within_width() {
    let mut font_system = test_font_system();
    let text = "one two three four five six seven";
    let mut buffer = shaped_buffer(&mut font_system, text, 120.0, Wrap::WordOrGlyph);

    let lines_before = buffer.layout_runs().count();

    buffer
        .set_indent(Indent::first_line(40.0))
        .expect("indent fits");
    buffer.shape_until_scroll(&mut font_system, false);

    // The first line has less room, so wrapping must produce at least as many lines.
    let lines_after = buffer.layout_runs().count();
    assert!(
        lines_after >= lines_before,
        "indenting must not reduce the number of visual lines"
    );

    // No glyph may cross the right content boundary (with a subpixel tolerance).
    for run in buffer.layout_runs() {
        let right = run
            .glyphs
            .iter()
            .map(|g| g.x + g.w)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            right <= 120.5,
            "glyph at {right} overflows the 120px content width"
        );
    }
}

#[test]
fn indent_combines_with_center_and_right_alignment() {
    let mut font_system = test_font_system();
    let text = "one two three four five six seven eight";
    let width = 160.0f32;

    for align in [Align::Left, Align::Right, Align::Center, Align::End] {
        let mut buffer = shaped_buffer(&mut font_system, text, width, Wrap::Word);
        buffer.set_text(
            text,
            &Attrs::new().family(Family::Name(FONT)),
            Shaping::Advanced,
            Some(align),
        );
        buffer
            .set_indent(Indent {
                first_line: 20.0,
                hanging: 10.0,
            })
            .expect("indent fits");
        buffer.shape_until_scroll(&mut font_system, false);

        for (i, run) in buffer.layout_runs().enumerate() {
            let shift = if i == 0 { 20.0 } else { 10.0 };
            let box_left = shift;
            let min_x = run.glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min);
            let max_right = run
                .glyphs
                .iter()
                .map(|g| g.x + g.w)
                .fold(f32::NEG_INFINITY, f32::max);

            // The indent moves the start edge inward; the wrap limit guarantees
            // nothing crosses the paragraph's end edge.
            assert!(
                min_x >= box_left - 0.01,
                "{align:?} line {i}: min {min_x} < {box_left}"
            );
            assert!(
                max_right <= width + 0.01,
                "{align:?} line {i}: max {max_right} > {width}"
            );

            match align {
                Align::Left => {
                    // Start edge sits exactly on the indented edge.
                    assert!(approx_eq(min_x, box_left), "{align:?} line {i}");
                }
                Align::Right | Align::End => {
                    // End edge stays at the paragraph edge.
                    assert!(approx_eq(max_right, width), "{align:?} line {i}");
                }
                Align::Center => {
                    // Centered within the box [indent, width].
                    let slack = width - box_left - run.line_w;
                    let expected = box_left + slack / 2.0;
                    assert!(
                        approx_eq(min_x, expected),
                        "{align:?} line {i}: {min_x} vs {expected}"
                    );
                }
                Align::Justified => unreachable!(),
            }
        }
    }
}

#[test]
fn indent_mirrors_for_rtl_paragraphs() {
    let mut db = fontdb::Database::new();
    db.load_font_data(std::fs::read("fonts/NotoSansArabic.ttf").unwrap());
    let mut font_system = FontSystem::new_with_locale_and_db("ar".into(), db);

    let text = "لكن لا بد أن أوضح لك أن كل هذه الأفكار المغلوطة حول استنكار النشوة ";
    let mut buffer = Buffer::new(&mut font_system, Metrics::new(16.0, 24.0));
    buffer.set_size(Some(140.0), Some(1000.0));
    buffer.set_wrap(Wrap::Word);
    buffer.set_text(text, &Attrs::new(), Shaping::Advanced, None);
    buffer.shape_until_scroll(&mut font_system, false);
    assert!(buffer.is_rtl(0).unwrap(), "test paragraph must be RTL");

    let plain_first_right = last_glyph_right(&buffer, 0);
    let plain_continuation_right = last_glyph_right(&buffer, 1);

    buffer
        .set_indent(Indent {
            first_line: 25.0,
            hanging: 12.0,
        })
        .expect("indent fits");
    buffer.shape_until_scroll(&mut font_system, false);

    // In RTL the start edge is on the right: indentation shifts it left.
    let indented_first_right = last_glyph_right(&buffer, 0);
    let indented_continuation_right = last_glyph_right(&buffer, 1);
    assert!(
        approx_eq(indented_first_right, plain_first_right - 25.0),
        "RTL first line should shift 25px left"
    );
    assert!(
        approx_eq(indented_continuation_right, plain_continuation_right - 12.0),
        "RTL continuation line should shift 12px left"
    );

    // Nothing extends past the right edge of the content area.
    for run in buffer.layout_runs() {
        let right = run
            .glyphs
            .iter()
            .map(|g| g.x + g.w)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(right <= 140.5, "RTL overflow at {right}");
    }
}

#[test]
fn indent_with_tabs_and_mixed_bidi_is_stable() {
    let mut font_system = test_font_system();
    // Tabs plus an embedded RTL run inside an LTR paragraph.
    let text = "a\tb\tc \u{05D0}\u{05D1}\u{05D2} d\t e f g h i j k l m n o p";
    let mut buffer = shaped_buffer(&mut font_system, text, 150.0, Wrap::WordOrGlyph);

    buffer
        .set_indent(Indent {
            first_line: 24.0,
            hanging: 8.0,
        })
        .expect("indent fits");
    buffer.shape_until_scroll(&mut font_system, false);

    let snapshot: Vec<(usize, Vec<(usize, f32)>)> = buffer
        .layout_runs()
        .map(|run| {
            (
                run.line_i,
                run.glyphs
                    .iter()
                    .map(|g| (g.start, g.x + g.w))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();

    // Relaying out must be deterministic (cached and uncached agree).
    buffer.shape_until_scroll(&mut font_system, false);
    let again: Vec<(usize, Vec<(usize, f32)>)> = buffer
        .layout_runs()
        .map(|run| {
            (
                run.line_i,
                run.glyphs
                    .iter()
                    .map(|g| (g.start, g.x + g.w))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    assert_eq!(snapshot, again, "layout must be stable across relayouts");

    // Every line remains inside the content width.
    for run in buffer.layout_runs() {
        let right = run
            .glyphs
            .iter()
            .map(|g| g.x + g.w)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(right <= 150.5, "overflow at {right} with tabs/bidi");
    }
}

#[test]
fn negative_and_non_finite_indents_are_rejected() {
    let mut font_system = test_font_system();
    let mut buffer = shaped_buffer(&mut font_system, "hello world", 100.0, Wrap::Word);

    let bad = [
        Indent::new(-1.0, 0.0),
        Indent::new(0.0, -5.0),
        Indent::new(f32::NAN, 0.0),
        Indent::new(0.0, f32::INFINITY),
    ];
    for indent in bad {
        let err = buffer.set_indent(indent).unwrap_err();
        assert!(
            matches!(err, IndentError::Negative { .. }),
            "expected Negative error, got {err:?}"
        );
    }

    // A rejected indent leaves the buffer untouched: still zero indentation.
    for line in &buffer.lines {
        assert_eq!(line.indent(), Indent::NONE);
    }

    // The same rule applies directly to a BufferLine.
    let err = buffer.lines[0]
        .set_indent(Indent::hanging(-2.0))
        .unwrap_err();
    assert!(matches!(err, IndentError::Negative { .. }));
}

#[test]
fn indent_wider_than_content_is_rejected_with_distinct_error() {
    let mut font_system = test_font_system();
    let mut buffer = shaped_buffer(&mut font_system, "hello world", 100.0, Wrap::Word);

    // Equal to the width: no room left for text.
    let err = buffer.set_indent(Indent::first_line(100.0)).unwrap_err();
    match err {
        IndentError::TooWide { indent, width } => {
            assert_eq!(indent.first_line, 100.0);
            assert_eq!(width, 100.0);
        }
        other => panic!("expected TooWide, got {other:?}"),
    }

    let err = buffer.set_indent(Indent::hanging(120.0)).unwrap_err();
    assert!(matches!(err, IndentError::TooWide { .. }));

    // Values strictly smaller than the width are accepted.
    buffer
        .set_indent(Indent::first_line(99.5))
        .expect("99.5px fits in 100px");

    // Without a width constraint, large finite indents are allowed.
    buffer.set_size(None, None);
    buffer
        .set_indent(Indent::first_line(10_000.0))
        .expect("unconstrained width accepts any finite indent");

    // validate_indent reports the same distinguishable error after shrinking.
    buffer.set_size(Some(50.0), None);
    let err = buffer.validate_indent(Indent::hanging(60.0)).unwrap_err();
    assert!(matches!(err, IndentError::TooWide { .. }));
}

#[test]
fn zero_indent_leaves_layout_bit_for_bit_identical() {
    let mut font_system = test_font_system();
    let text = "lorem ipsum dolor sit amet\nconsectetur adipiscing elit sed do";
    let mut buffer = shaped_buffer(&mut font_system, text, 130.0, Wrap::WordOrGlyph);

    let snapshot: Vec<(f32, f32, Vec<(usize, usize, f32)>)> = buffer
        .layout_runs()
        .map(|run| {
            (
                run.line_w,
                run.line_y,
                run.glyphs
                    .iter()
                    .map(|g| (g.start, g.end, g.x))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();

    buffer.set_indent(Indent::NONE).unwrap();
    buffer.shape_until_scroll(&mut font_system, false);

    let after: Vec<(f32, f32, Vec<(usize, usize, f32)>)> = buffer
        .layout_runs()
        .map(|run| {
            (
                run.line_w,
                run.line_y,
                run.glyphs
                    .iter()
                    .map(|g| (g.start, g.end, g.x))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();

    assert_eq!(snapshot, after, "zero indent must not move any glyph");
}

#[test]
fn rtl_indent_combines_with_every_alignment() {
    let mut db = fontdb::Database::new();
    db.load_font_data(std::fs::read("fonts/NotoSansArabic.ttf").unwrap());
    let mut font_system = FontSystem::new_with_locale_and_db("ar".into(), db);

    let text = "لكن لا بد أن أوضح لك أن كل هذه الأفكار المغلوطة حول استنكار النشوة ";
    let width = 160.0f32;
    let shift = 20.0f32;

    for align in [
        Align::Left,
        Align::Right,
        Align::Center,
        Align::End,
        Align::Justified,
    ] {
        let mut buffer = Buffer::new(&mut font_system, Metrics::new(16.0, 24.0));
        buffer.set_size(Some(width), Some(1000.0));
        buffer.set_wrap(Wrap::Word);
        buffer.set_text(text, &Attrs::new(), Shaping::Advanced, Some(align));
        buffer
            .set_indent(Indent::first_line(shift))
            .expect("indent fits");
        buffer.shape_until_scroll(&mut font_system, false);

        for (i, run) in buffer.layout_runs().enumerate() {
            let line_shift = if i == 0 { shift } else { 0.0 };
            let min_x = run.glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min);
            let max_right = run
                .glyphs
                .iter()
                .map(|g| g.x + g.w)
                .fold(f32::NEG_INFINITY, f32::max);

            // RTL start edge is the right one: the indent moves it to the left.
            assert!(
                max_right <= width - line_shift + 0.01,
                "{align:?} RTL line {i}: right edge {max_right} exceeds {}",
                width - line_shift
            );
            // Nothing escapes the paragraph's left edge either.
            assert!(min_x >= -0.01, "{align:?} RTL line {i}: left edge {min_x}");

            match align {
                // Natural RTL alignment keeps the (indented) right edge flush.
                Align::Right | Align::Justified => {
                    assert!(
                        approx_eq(max_right, width - line_shift),
                        "{align:?} RTL line {i}: {max_right}"
                    );
                }
                // Left/End alignment pushes content against the left edge.
                Align::Left | Align::End => {
                    assert!(approx_eq(min_x, 0.0), "{align:?} RTL line {i}: {min_x}");
                }
                Align::Center => {
                    let slack = width - line_shift - run.line_w;
                    let expected = slack / 2.0;
                    assert!(
                        approx_eq(min_x, expected),
                        "{align:?} RTL line {i}: {min_x} vs {expected}"
                    );
                }
            }
        }
    }
}

#[test]
fn indent_with_justification_fills_indented_box() {
    let mut font_system = test_font_system();
    let text = "one two three four five six seven eight nine ten eleven twelve";
    let mut buffer = shaped_buffer(&mut font_system, text, 180.0, Wrap::Word);
    buffer.set_text(
        text,
        &Attrs::new().family(Family::Name(FONT)),
        Shaping::Advanced,
        Some(Align::Justified),
    );
    buffer
        .set_indent(Indent {
            first_line: 24.0,
            hanging: 12.0,
        })
        .expect("indent fits");
    buffer.shape_until_scroll(&mut font_system, false);

    let runs: Vec<_> = buffer.layout_runs().collect();
    assert!(runs.len() >= 2, "sample must wrap");
    for (i, run) in runs.iter().enumerate() {
        let shift = if i == 0 { 24.0 } else { 12.0 };
        let min_x = run.glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min);
        let max_right = run
            .glyphs
            .iter()
            .map(|g| g.x + g.w)
            .fold(f32::NEG_INFINITY, f32::max);
        // Start edge keeps the indent; justification never crosses the
        // paragraph's end edge.
        assert!(min_x >= shift - 0.01, "line {i} start {min_x} < {shift}");
        assert!(
            max_right <= 180.0 + 0.01,
            "line {i} end {max_right} overflows"
        );
        if i < runs.len() - 1 {
            // Non-last lines are justified: they reach the end edge.
            assert!(
                approx_eq(max_right, 180.0),
                "justified line {i} should fill to end edge, got {max_right}"
            );
        }
    }
}

#[test]
fn first_and_hanging_indent_compose() {
    let mut font_system = test_font_system();
    let text = "one two three four five six seven eight nine ten eleven";
    let mut buffer = shaped_buffer(&mut font_system, text, 120.0, Wrap::Word);

    let baseline: Vec<f32> = buffer
        .layout_runs()
        .map(|run| run.glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min))
        .collect();

    buffer
        .set_indent(Indent::new(30.0, 10.0))
        .expect("indents fit");
    buffer.shape_until_scroll(&mut font_system, false);

    let indented: Vec<f32> = buffer
        .layout_runs()
        .map(|run| run.glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min))
        .collect();

    assert!(approx_eq(indented[0], baseline[0] + 30.0));
    for line in 1..baseline.len() {
        assert!(approx_eq(indented[line], baseline[line] + 10.0));
    }
}

#[test]
fn zero_indent_is_valid_even_on_zero_width_buffer() {
    let mut font_system = test_font_system();
    let mut buffer = shaped_buffer(&mut font_system, "hello", 1.0, Wrap::Word);
    buffer.set_size(Some(0.0), Some(1000.0));

    // Clearing indentation must always work.
    buffer
        .set_indent(Indent::NONE)
        .expect("zero indent is always valid");

    // Any positive indent still has no room on a zero-width buffer.
    let err = buffer
        .set_indent(Indent::first_line(1.0))
        .expect_err("zero width cannot host a positive indent");
    assert!(matches!(err, IndentError::TooWide { .. }));
}

#[test]
fn indent_survives_editor_paragraph_split() {
    use cosmic_text::{Action, Edit, Editor, Motion};

    let mut font_system = test_font_system();
    let buffer = Buffer::new(&mut font_system, Metrics::new(16.0, 24.0));
    let mut editor = Editor::new(buffer);
    editor.with_buffer_mut(|buf| {
        buf.set_size(Some(200.0), Some(1000.0));
        buf.set_text(
            "first paragraph content here",
            &Attrs::new().family(Family::Name(FONT)),
            Shaping::Advanced,
            None,
        );
        buf.set_indent(Indent::first_line(22.0))
            .expect("indent fits");
    });

    // Move one character right and press Enter, splitting the paragraph.
    editor.action(&mut font_system, Action::Motion(Motion::Next));
    editor.action(&mut font_system, Action::Enter);

    editor.with_buffer(|buf| {
        assert_eq!(buf.lines.len(), 2);
        // Both resulting paragraphs retain the paragraph-level indentation.
        for line in &buf.lines {
            assert_eq!(line.indent(), Indent::first_line(22.0));
        }
    });
}
