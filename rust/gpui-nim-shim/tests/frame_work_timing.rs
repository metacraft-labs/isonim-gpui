//! The measurement the cached-view boundary threshold is chosen from
//! (`cached_views::DEFAULT_BOUNDARY_MIN_NODES`). Not a pass/fail test: it
//! prints frame times, so it is `#[ignore]`d and run by hand:
//!
//!     cargo test -p gpui-nim-shim --features gpui-backend \
//!       --test frame_work_timing -- --ignored --nocapture
//!
//! A list of 1,000 rows, each a subtree of `k` plan nodes (a row holding
//! `k - 1` nodes), is drawn in a real GPUI test-platform window; one row
//! changes per frame. For each `k` the change frame is timed with every row
//! behind a cached view (threshold 1) and with none (threshold off). The
//! smallest `k` at which caching is no slower is the threshold. The one
//! stand-in is the test platform, which draws into no surface; both sides of
//! every comparison run on it.

#![cfg(feature = "gpui-backend")]

use std::ffi::CString;

use gpui::TestAppContext;
use gpui_nim_shim::cached_views::{set_boundary_min_nodes, DEFAULT_BOUNDARY_MIN_NODES};
use gpui_nim_shim::gpui_app::NimRootView;
use gpui_nim_shim::{
    gpui_append_child, gpui_create_element, gpui_create_text_node, gpui_reset_tree,
    gpui_set_attribute, gpui_set_root_element, gpui_set_style, gpui_set_text_content,
    GpuiElement,
};

fn el(tag: &str, styles: &[(&str, &str)]) -> *mut GpuiElement {
    let tag = CString::new(tag).unwrap();
    let e = gpui_create_element(tag.as_ptr());
    for (k, v) in styles {
        let k = CString::new(*k).unwrap();
        let v = CString::new(*v).unwrap();
        gpui_set_style(e, k.as_ptr(), v.as_ptr());
    }
    e
}

/// Rows of `k` plan nodes: the row, then `k - 1` text nodes in it. Returns
/// what a change frame changes: each row's first text node, or the row
/// itself when it holds none.
fn build(rows: usize, k: usize) -> Vec<*mut GpuiElement> {
    gpui_reset_tree();
    let root = el("root", &[("display", "flex"), ("flex-direction", "column"), ("width", "800px"), ("height", "600px")]);
    let list = el("div", &[("display", "flex"), ("flex-direction", "column"), ("width", "800px"), ("height", "600px"), ("overflow", "hidden")]);
    gpui_append_child(root, list);
    let mut firsts = Vec::new();
    for i in 0..rows {
        let row = el("div", &[("display", "flex"), ("flex-direction", "row"), ("width", "800px"), ("height", "20px"), ("flex-shrink", "0")]);
        for j in 0..k.saturating_sub(1) {
            let t = gpui_create_text_node(CString::new(format!("{i}.{j} ")).unwrap().as_ptr());
            gpui_append_child(row, t);
            if j == 0 {
                firsts.push(t);
            }
        }
        gpui_append_child(list, row);
        if k == 1 {
            firsts.push(row);
        }
    }
    gpui_set_root_element(root);
    firsts
}

fn median_change_frame_us(cx: &mut TestAppContext, k: usize, min_nodes: usize) -> u64 {
    set_boundary_min_nodes(min_nodes);
    let texts = build(1000, k);
    let (view, vcx) = cx.add_window_view(|_window, _cx| NimRootView::new());
    vcx.run_until_parked();
    let mut times = Vec::new();
    for n in 0..31 {
        let t = CString::new(format!("c{n}")).unwrap();
        let target = texts[(n * 37) % texts.len()];
        let started = std::time::Instant::now();
        if k == 1 {
            // A row with nothing in it changes an attribute, which changes
            // nothing it draws: the frame still redraws the row.
            let name = CString::new("data-change").unwrap();
            gpui_set_attribute(target, name.as_ptr(), t.as_ptr());
        } else {
            gpui_set_text_content(target, t.as_ptr());
        }
        vcx.update(|_window, app| view.update(app, |_, cx| cx.notify()));
        vcx.run_until_parked();
        times.push(started.elapsed().as_micros() as u64);
    }
    times.sort_unstable();
    times[times.len() / 2]
}

#[test]
#[ignore]
fn boundary_threshold_measurement() {
    gpui::run_test(
        1,
        &[],
        0,
        &mut |dispatcher, _seed| {
            let mut cx = TestAppContext::build(dispatcher.clone(), Some("frame_work_timing"));
            for k in [1usize, 2, 3, 4, 6, 8] {
                let cached = median_change_frame_us(&mut cx, k, 1);
                let plain = median_change_frame_us(&mut cx, k, usize::MAX);
                eprintln!("timing k={k}: cached {cached} us, plain {plain} us (1000 rows, median of 31 change frames)");
            }
        },
        None,
    );
    set_boundary_min_nodes(DEFAULT_BOUNDARY_MIN_NODES);
}
