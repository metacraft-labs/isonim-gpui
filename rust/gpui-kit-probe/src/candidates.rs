//! PLAT-42 — the editor decision's cells, one per feature. See Cargo.toml.
//!
//! Criteria K1-K5 were digest-frozen (codetracer-specs 9dde298d) BEFORE this
//! file was written. Each function below is the smallest honest attempt to
//! draw one surface under one candidate; whether it compiles is the answer.
#![allow(dead_code, unused_imports, unused_variables)]

use gpui::{App, Bounds, HighlightStyle, Hsla, Pixels, Point};
use gpui_base::input::{EditorState, TextDecoration, TextDecorationCollection};
use std::ops::Range;

/// B's overlay geometry, public API only: where on screen is buffer line `line`?
/// Every B surface is drawn by the host at this point, so every B cell needs it.
#[cfg(any(feature = "b-pointer", feature = "b-status", feature = "b-values", feature = "b-flow"))]
fn b_row_origin(state: &EditorState, line: usize) -> Option<Point<Pixels>> {
    let rows = state.visible_row_range()?;
    let lh = state.line_height()?;
    let scroll = state.scroll_offset();
    let text = state.text_bounds()?;
    // ROW == LINE, which holds only while soft wrap and folding are OFF. The
    // fold-aware mapping is measured as its OWN cell (`b-foldaware`), so that
    // one unreachable call cannot fail all four surfaces for one reason and
    // hand back one verdict wearing four labels.
    let display_row = line;
    if !rows.contains(&display_row) {
        return None;
    }
    Some(Point {
        x: text.origin.x,
        y: text.origin.y + lh * (display_row as f32) + scroll.y,
    })
}

/// Surface 1 under B: the execution line's background, plus a gutter marker.
#[cfg(feature = "b-pointer")]
pub fn b_pointer(state: &EditorState, deco: &TextDecorationCollection,
                 line_bytes: Range<usize>, line: usize, bg: Hsla, cx: &mut App) {
    // The highlight: a public decoration over the line's byte range. This paints
    // the TEXT range, not the full row width — recorded, not hidden.
    deco.set(vec![TextDecoration::new(line_bytes, HighlightStyle {
        background_color: Some(bg),
        ..Default::default()
    })], cx);
    // The marker: drawn by the host at the row's origin.
    let _ = b_row_origin(state, line);
}

/// Surface 2 under B: a per-line status mark in the gutter, host-drawn.
#[cfg(feature = "b-status")]
pub fn b_status(state: &EditorState, line: usize) -> Option<Point<Pixels>> {
    b_row_origin(state, line)
}

/// Surface 3 under B: an inline value drawn AFTER the line's code. Needs the
/// x at which the line's text ends; `text_bounds` is the whole text area.
#[cfg(feature = "b-values")]
pub fn b_values(state: &EditorState, line: usize) -> Option<Point<Pixels>> {
    let origin = b_row_origin(state, line)?;
    let end_x = state.line_end_x(line)?;
    Some(Point { x: end_x, y: origin.y })
}

/// Surface 4 under B: a per-line flow annotation, host-drawn like a value.
#[cfg(feature = "b-flow")]
pub fn b_flow(state: &EditorState, line: usize) -> Option<Point<Pixels>> {
    b_row_origin(state, line)
}

/// A: the gutter itself — where the editor lays out its gutter column.
#[cfg(any(feature = "a-pointer", feature = "a-status"))]
fn a_gutter(state: &EditorState, window: &gpui::Window) -> Bounds<Pixels> {
    gpui_base::input::editor::element::editor_gutter_bounds(state, window)
}

#[cfg(feature = "a-pointer")]
pub fn a_pointer(state: &EditorState, window: &gpui::Window) -> Bounds<Pixels> {
    a_gutter(state, window)
}

#[cfg(feature = "a-status")]
pub fn a_status(state: &EditorState, window: &gpui::Window) -> Bounds<Pixels> {
    a_gutter(state, window)
}

/// A: inline values as an inline completion inside the editor's own layout.
#[cfg(feature = "a-values")]
pub fn a_values(state: &mut EditorState) {
    let _ = &state.extras.inline_completion;
}

/// A: the flow overlay as a BLOCK between rows — needs a block layer in the
/// display map.
#[cfg(feature = "a-flow")]
pub fn a_flow(state: &EditorState) {
    let _ = state.display_map().block_map();
}

/// B, fold- and wrap-aware placement: buffer line -> display row through the
/// editor's own display map. The milestone credits B with this; whether a host
/// can OBTAIN the display map from an `EditorState` is what compiles or not.
#[cfg(feature = "b-foldaware")]
pub fn b_foldaware(state: &EditorState, line: usize) -> usize {
    state.display_map().buffer_line_to_display_row(line)
}
