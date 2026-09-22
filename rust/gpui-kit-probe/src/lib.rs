//! PLAT-42 — does gpui-kit's type graph unify with this workspace's?
//!
//! **THE COMPILE IS THE ASSERTION, AND IT ONLY ASSERTS IF SOMETHING ACTUALLY
//! CROSSES THE BOUNDARY.**
//!
//! The first version of this probe was `pub fn unifies(_w: &mut gpui::Window) {}`
//! and it was VACUOUS: it accepted our `Window`, did nothing with it, and
//! compiled green — as it would have if `gpui-kit` had resolved to a different
//! `gpui-pre` entirely, because nothing ever handed our type to their code. A
//! probe whose subject never meets its object measures the build, not the
//! property (`Verification-Harness-Traps.md` §4 — a check that touches nothing
//! is satisfied by everything).
//!
//! Both functions below cross, in both directions:
//!
//!   * values built through OUR `gpui::` path are passed to gpui-kit functions
//!     that demand THEIRS;
//!   * a value gpui-kit RETURNS is then consumed as ours.
//!
//! If the two packages were distinct, `Pixels`, `Bounds<Pixels>` and `Hsla`
//! would each be two unrelated types and rustc would reject every line here.
//!
//! **WHAT THIS PROBE DOES NOT CLAIM.** It says the graphs unify, nothing more.
//! It does not say gpui-kit's editor is adoptable — the first attempt at this
//! file tried to reach `gpui_base::touch_selection::TouchSelection` and was
//! refused because that module is PRIVATE, which is itself evidence about
//! candidate A rather than about linking.

use gpui::{px, Bounds, Hsla, Pixels, Point, Size};

/// Crossing 1, through `gpui-base`: our `Pixels` and `Bounds<Pixels>` go in,
/// their `Option<Pixels>` comes back, and the result is used as OURS.
pub fn unifies_base(y: Pixels, bounds: Bounds<Pixels>) -> Pixels {
    match gpui_base::AutoScroll::compute_delta(y, bounds) {
        // `d` is whatever `gpui-base` returned. Adding it to a `Pixels` built
        // here is the second half of the crossing: if the graph had split,
        // this addition would be between two unrelated types.
        Some(d) => d + y,
        None => y,
    }
}

/// Crossing 2, through `gpui-component`: three of OUR `Hsla` values go into
/// their constructor, so the claim does not rest on one function in one crate.
pub fn unifies_component(color: Hsla, fg: Hsla, border: Hsla) -> gpui_component::tag::Tag {
    gpui_component::tag::Tag::custom(color, fg, border)
}

/// A value built entirely through our path, to hand to the functions above —
/// so a caller needs nothing from gpui-kit to exercise the crossing.
pub fn our_bounds() -> Bounds<Pixels> {
    Bounds {
        // `px()` rather than `Pixels(..)`: the tuple struct's field is
        // private, which is gpui's own encapsulation and not a graph problem.
        origin: Point { x: px(0.0), y: px(0.0) },
        size: Size { width: px(100.0), height: px(100.0) },
    }
}
