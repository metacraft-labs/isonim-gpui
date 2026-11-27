//! A row scrolled into view takes the pointer where it is drawn.
//!
//! A host scrolls a list the way CodeTracer's Variables pane does: the rows'
//! container gets a negative `margin-top` inside a parent that clips
//! (`overflow: hidden`). A row that was below the clip before the scroll is
//! then drawn inside it, and a press there must reach that row's listeners,
//! as a press on a row that was in view all along does.
//!
//! The pane, styled as CodeTracer's Variables pane is: a 188 px table that
//! clips, and in it a column of 24 px rows that does not shrink, its rows
//! aligned to the start. Scrolled by 101 px, row 5 (top 120) is in view
//! before and after (at 19), and row 10 (top 240) is clipped before and in
//! view after (at 139). Each case presses both, after the scroll, in a real
//! GPUI test-platform window (no surface; layout, prepaint, paint and
//! hit-testing are a window's), once with every eligible subtree behind a
//! cached view and once with none. Row 5 is the control: a case in which no
//! press reached anything fails there first.
//!
//! The scroll is applied two ways: as a style change on the live rows (an
//! incremental host), and as a new column of new rows put in the old one's
//! place (a host that draws the pane again, as CodeTracer's does).

#![cfg(feature = "gpui-backend")]

use std::ffi::CString;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui::TestAppContext;
use gpui_nim_shim::cached_views::{set_boundary_min_nodes, DEFAULT_BOUNDARY_MIN_NODES};
use gpui_nim_shim::gpui_app::NimRootView;
use gpui_nim_shim::layout_bounds::{bounds_of, gpui_layout_bounds_reset};
use gpui_nim_shim::{
    gpui_add_event_listener, gpui_append_child, gpui_create_element, gpui_create_text_node,
    gpui_node_id, gpui_remove_child, gpui_reset_tree, gpui_set_attribute, gpui_set_root_element,
    gpui_set_style, GpuiElement,
};
use serial_test::serial;

const ROWS: usize = 20;
const SCROLL: &str = "-101px";

fn run_gpui_test(name: &'static str, min_nodes: usize, f: fn(&mut TestAppContext)) {
    set_boundary_min_nodes(min_nodes);
    gpui::run_test(
        1,
        &[],
        0,
        &mut |dispatcher, _seed| {
            let mut cx = TestAppContext::build(dispatcher.clone(), Some(name));
            f(&mut cx);
        },
        None,
    );
    set_boundary_min_nodes(DEFAULT_BOUNDARY_MIN_NODES);
}

fn el(tag: &str, styles: &[(&str, &str)]) -> *mut GpuiElement {
    let tag = CString::new(tag).unwrap();
    let e = gpui_create_element(tag.as_ptr());
    for (k, v) in styles {
        style(e, k, v);
    }
    let n = CString::new("data-layout-probe").unwrap();
    let v = CString::new("").unwrap();
    gpui_set_attribute(e, n.as_ptr(), v.as_ptr());
    e
}

fn style(node: *mut GpuiElement, prop: &str, value: &str) {
    let p = CString::new(prop).unwrap();
    let v = CString::new(value).unwrap();
    gpui_set_style(node, p.as_ptr(), v.as_ptr());
}

/// Which row the last click reached (its index + 1; 0 for none).
static CLICKED: AtomicU64 = AtomicU64::new(0);
extern "C" fn on_click_5(_p: *const gpui_nim_shim::input::GpuiEventPayload) {
    CLICKED.store(6, Ordering::SeqCst);
}
extern "C" fn on_click_10(_p: *const gpui_nim_shim::input::GpuiEventPayload) {
    CLICKED.store(11, Ordering::SeqCst);
}

/// The rows' column, scrolled by `margin_top`, with listeners on rows 5 and
/// 10. Answers the column and its rows.
fn column(margin_top: &str) -> (*mut GpuiElement, Vec<*mut GpuiElement>) {
    let scroller = el(
        "div",
        &[
            ("display", "flex"),
            ("flex-direction", "column"),
            ("flex-shrink", "0"),
            ("items", "start"),
            ("margin-top", margin_top),
            ("margin-left", "0px"),
        ],
    );
    let mut rows = Vec::new();
    for i in 0..ROWS {
        let row = el(
            "div",
            &[
                ("display", "flex"),
                ("items", "center"),
                ("flex-shrink", "0"),
                ("height", "24px"),
                ("white-space", "nowrap"),
                ("overflow", "hidden"),
                ("gap", "8px"),
            ],
        );
        let a = el("span", &[("width", "60px"), ("flex-shrink", "0")]);
        let t = CString::new(format!("row{i}:")).unwrap();
        gpui_append_child(a, gpui_create_text_node(t.as_ptr()));
        let b = el("span", &[]);
        let t = CString::new(format!("value {i}")).unwrap();
        gpui_append_child(b, gpui_create_text_node(t.as_ptr()));
        gpui_append_child(row, a);
        gpui_append_child(row, b);
        gpui_append_child(scroller, row);
        rows.push(row);
    }
    let click = CString::new("click").unwrap();
    gpui_add_event_listener(rows[5], click.as_ptr(), on_click_5);
    gpui_add_event_listener(rows[10], click.as_ptr(), on_click_10);
    (scroller, rows)
}

/// The window's tree: a fixed pane, the table that clips, the column.
fn build() -> (*mut GpuiElement, *mut GpuiElement, Vec<*mut GpuiElement>) {
    gpui_reset_tree();
    let root = el(
        "root",
        &[
            ("display", "flex"),
            ("flex-direction", "column"),
            ("width", "640px"),
            ("height", "480px"),
        ],
    );
    let pane = el(
        "div",
        &[
            ("display", "flex"),
            ("flex-direction", "column"),
            ("width", "400px"),
            ("height", "200px"),
            ("flex-shrink", "0"),
        ],
    );
    gpui_append_child(root, pane);
    let table = el(
        "div",
        &[
            ("display", "flex"),
            ("flex-direction", "column"),
            ("overflow", "hidden"),
            ("margin-top", "12px"),
            ("padding-left", "8px"),
            ("flex-grow", "1"),
        ],
    );
    gpui_append_child(pane, table);
    let (scroller, rows) = column("0px");
    gpui_append_child(table, scroller);
    gpui_set_root_element(root);
    (table, scroller, rows)
}

fn frame(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<NimRootView>) {
    cx.update(|_window, app| view.update(app, |_, cx| cx.notify()));
    cx.run_until_parked();
}

/// Click the middle of `row` where the window laid it out, and answer which
/// row's listener took it (index + 1; 0 for none).
fn click(cx: &mut gpui::VisualTestContext, row: *mut GpuiElement) -> u64 {
    let b = bounds_of(gpui_node_id(row)).expect("the row was laid out");
    let at = gpui::point(gpui::px(b.x + 10.0), gpui::px(b.y + b.h / 2.0));
    CLICKED.store(0, Ordering::SeqCst);
    cx.simulate_click(at, gpui::Modifiers::default());
    cx.run_until_parked();
    CLICKED.load(Ordering::SeqCst)
}

fn scrolled_rows(cx: &mut TestAppContext, rebuild: bool) {
    let (table, scroller, rows) = build();
    gpui_layout_bounds_reset();
    let (view, vcx) = cx.add_window_view(|_window, _cx| NimRootView::new());
    vcx.run_until_parked();
    frame(vcx, &view);
    let rows = if rebuild {
        gpui_remove_child(table, scroller);
        let (scroller, rows) = column(SCROLL);
        gpui_append_child(table, scroller);
        rows
    } else {
        style(scroller, "margin-top", SCROLL);
        rows
    };
    frame(vcx, &view);
    let t = bounds_of(gpui_node_id(table)).expect("the table was laid out");
    let r10 = bounds_of(gpui_node_id(rows[10])).expect("row 10 was laid out");
    assert!(
        r10.y >= t.y && r10.y + r10.h <= t.y + t.h,
        "row 10 is drawn inside the table after the scroll: row {r10:?}, table {t:?}"
    );
    assert_eq!(
        click(vcx, rows[5]),
        6,
        "control: row 5, in view before and after, takes its click"
    );
    assert_eq!(
        click(vcx, rows[10]),
        11,
        "row 10, scrolled into view, takes its click"
    );
}

#[test]
#[serial]
fn a_row_scrolled_in_by_a_style_change_takes_a_click() {
    run_gpui_test("scrolled_style_plain", usize::MAX, |cx| {
        scrolled_rows(cx, false)
    });
}

#[test]
#[serial]
fn a_row_scrolled_in_by_a_style_change_takes_a_click_cached() {
    run_gpui_test("scrolled_style_cached", 1, |cx| scrolled_rows(cx, false));
}

#[test]
#[serial]
fn a_row_of_a_column_drawn_again_scrolled_takes_a_click() {
    run_gpui_test("scrolled_rebuild_plain", usize::MAX, |cx| {
        scrolled_rows(cx, true)
    });
}

#[test]
#[serial]
fn a_row_of_a_column_drawn_again_scrolled_takes_a_click_cached() {
    run_gpui_test("scrolled_rebuild_cached", 1, |cx| scrolled_rows(cx, true));
}
