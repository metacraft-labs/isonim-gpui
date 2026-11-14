//! Text shaped by the PLATFORM's own text system: the shim's
//! `font-variant-numeric` and `letter-spacing` reach the glyphs a real
//! shaper produces, not only the plan.
//!
//! These cases moved here from `gpui_rendering.rs` unchanged in what they
//! assert. They need a real text system, which gpui hands out only through
//! a platform (`gpui_platform::current_platform`), and that is where they
//! part from the rest of that file: `TestAppContext` installs gpui's
//! `NoopTextSystem` (see `digit_advances` below for why that cannot test
//! this).
//!
//! # Why `harness = false`
//!
//! On macOS `current_platform` builds `MacPlatform`, and
//! `MacPlatform::new` takes a `MainThreadMarker`: it panics with
//! "Mac platform not created on main thread" anywhere else
//! (`gpui-pre-macos-0.3.5/src/platform.rs:210`), headless or not. libtest
//! runs every `#[test]` on a worker thread, so as `#[test]`s these cases
//! panicked there before asserting anything. This file is its own `main`
//! (declared `harness = false` in `Cargo.toml`, as `test_mac_window_frame`
//! is, for the same reason), so every case runs on the process's main
//! thread, one after another. On Linux the platform is the headless client
//! with the cosmic-text system the window uses, which any thread may build;
//! the cases run the same way there.
//!
//! `main` hands the cases to `main_thread_harness::run` (see
//! `tests/main_thread_harness/mod.rs`), which runs them in order, prints
//! libtest's lines, and fails `cargo test` if any case failed. Nothing
//! else runs in this process, so the `#[serial]` the cases carried in
//! `gpui_rendering.rs` has nothing left to order them against.

mod main_thread_harness;

use gpui::div;
use gpui_nim_shim::gpui_app::apply_styles_to_div;
use gpui_nim_shim::render_sync::GpuiStyles;

// ---------------------------------------------------------------------------
// `font-variant-numeric: tabular-nums` reaches the shaper.
//
// WHY A REAL TEXT SYSTEM. `TestAppContext` installs gpui's `NoopTextSystem`,
// which advances every character by 0.6 em and ignores font features, so
// a test through it would pass with or without the feature. This one shapes
// with the platform's own text system (`gpui_platform::current_platform`,
// headless: on Linux the cosmic-text system the window uses) and the design
// system's interface face, committed as a fixture beside its licence
// (`tests/fixtures/fonts/`, SIL OFL 1.1). The face is read from the fixture,
// not from the host, so the numbers below are the face's: Space Grotesk
// Regular's proportional digits run from 416 units ("1") to 641 ("0") of
// its 1000-unit em, and its `tnum` substitutes `*.tf` digits all 620 wide.
//
// THE CONTROL is the same element without the property: its digits must
// keep their different advances, which proves the fixture really is
// proportional by default and that equal advances come from the feature.
// ---------------------------------------------------------------------------

/// The advance of each character of `text`, set in the fixture face at
/// 100 px with the text style that `apply_styles_to_div` gives an element
/// carrying `styles` -- the font GPUI's text layout would ask for.
fn digit_advances(styles: &GpuiStyles, text: &str) -> Vec<f32> {
    use gpui::{Refineable, Styled};
    const FIXTURE: &[u8] = include_bytes!("fixtures/fonts/SpaceGrotesk-Regular.otf");
    let platform = gpui_platform::current_platform(true);
    let text_system = platform.text_system();
    text_system
        .add_fonts(vec![std::borrow::Cow::Borrowed(FIXTURE)])
        .expect("the fixture face loads");
    let mut el = apply_styles_to_div(div(), styles);
    let mut style = gpui::TextStyle::default();
    style.refine(el.text_style());
    let font = style.font();
    let font_id = text_system.font_id(&font).expect("the family resolves");
    let layout = text_system.layout_line(
        text,
        gpui::px(100.0),
        &[gpui::FontRun {
            len: text.len(),
            font_id,
        }],
    );
    let mut xs: Vec<(usize, f32)> = layout
        .runs
        .iter()
        .flat_map(|run| run.glyphs.iter())
        .map(|g| (g.index, f32::from(g.position.x)))
        .collect();
    xs.sort_by_key(|(i, _)| *i);
    assert_eq!(xs.len(), text.chars().count(), "one glyph per character");
    let mut ends: Vec<f32> = xs.iter().skip(1).map(|(_, x)| *x).collect();
    ends.push(f32::from(layout.width));
    xs.iter().zip(ends).map(|((_, x), end)| end - x).collect()
}

fn test_tabular_nums_sets_every_digit_one_advance() {
    let mut styles = GpuiStyles::default();
    styles.font_family = Some("Space Grotesk".into());

    // CONTROL: proportional by default ("1" 41.6 px, "0" 64.1 px at 100 px).
    let proportional = digit_advances(&styles, "0123456789");
    assert!(
        (proportional[0] - 64.1).abs() < 0.05 && (proportional[1] - 41.6).abs() < 0.05,
        "control: the fixture face's own digits are proportional, got {proportional:?}"
    );

    styles.font_variant_numeric = Some("tabular-nums".into());
    let tabular = digit_advances(&styles, "0123456789");
    for (digit, advance) in "0123456789".chars().zip(&tabular) {
        assert!(
            (advance - 62.0).abs() < 0.05,
            "with tabular-nums '{digit}' must be the tnum advance 62.0 px, got {advance} \
             (all: {tabular:?})"
        );
    }

    // `proportional-nums` asks for `pnum`, which is the face's default.
    styles.font_variant_numeric = Some("proportional-nums".into());
    assert_eq!(digit_advances(&styles, "0123456789"), proportional);
}

/// WHERE THE GLYPHS LAND, shaped by the platform's own text system with the
/// committed Space Grotesk fixture (see `digit_advances` for why a real
/// text system and this face). "SESSIONS" at 12px with the design system's
/// `letter-spacing: 0.03125em` (0.375px): every glyph moves right by 0.375px
/// per character before it, and the line is 3px (8 x 0.375) wider -- the
/// difference measured between the browser's 58px label and GPUI's 55px
/// before the shim drew the property.
fn test_letter_spaced_glyphs_move_by_the_spacing_of_the_characters_before_them() {
    use gpui::{Refineable, Styled};
    use gpui_nim_shim::gpui_app::{letter_spaced_xs, parse_letter_spacing};
    const FIXTURE: &[u8] = include_bytes!("fixtures/fonts/SpaceGrotesk-Regular.otf");
    let platform = gpui_platform::current_platform(true);
    let text_system = platform.text_system();
    text_system
        .add_fonts(vec![std::borrow::Cow::Borrowed(FIXTURE)])
        .expect("the fixture face loads");
    let mut styles = GpuiStyles::default();
    styles.font_family = Some("Space Grotesk".into());
    let mut el = apply_styles_to_div(div(), &styles);
    let mut style = gpui::TextStyle::default();
    style.refine(el.text_style());
    let font_id = text_system.font_id(&style.font()).expect("the family resolves");
    let text = "SESSIONS";
    let layout = text_system.layout_line(
        text,
        gpui::px(12.0),
        &[gpui::FontRun {
            len: text.len(),
            font_id,
        }],
    );
    let natural: Vec<f32> = layout
        .runs
        .iter()
        .flat_map(|r| r.glyphs.iter())
        .map(|g| f32::from(g.position.x))
        .collect();
    assert_eq!(natural.len(), text.len(), "one glyph per capital");

    // CONTROL: no spacing leaves every glyph where the shaper put it.
    let (xs0, w0) = letter_spaced_xs(&layout, text, 0.0);
    assert_eq!(xs0, natural);
    assert_eq!(w0, f32::from(layout.width));

    let spacing = parse_letter_spacing("0.03125em").expect("parses").to_px(12.0);
    assert_eq!(spacing, 0.375);
    let (xs, width) = letter_spaced_xs(&layout, text, spacing);
    for (k, (x, n)) in xs.iter().zip(&natural).enumerate() {
        assert!(
            (x - (n + k as f32 * 0.375)).abs() < 1e-4,
            "glyph {k} ('{}') at {x}, expected {} + {k} x 0.375",
            &text[k..k + 1],
            n
        );
    }
    assert!(
        (width - (f32::from(layout.width) + 3.0)).abs() < 1e-4,
        "the spaced line is 3px wider: {width} against {}",
        f32::from(layout.width)
    );
}

/// The cases, in the order they run.
const CASES: &[main_thread_harness::Case] = &[
    (
        "test_tabular_nums_sets_every_digit_one_advance",
        test_tabular_nums_sets_every_digit_one_advance,
    ),
    (
        "test_letter_spaced_glyphs_move_by_the_spacing_of_the_characters_before_them",
        test_letter_spaced_glyphs_move_by_the_spacing_of_the_characters_before_them,
    ),
];

fn main() {
    main_thread_harness::run(CASES);
}
