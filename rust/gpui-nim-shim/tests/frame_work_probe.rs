//! THE PROPORTIONALITY PROBE: one label changes in a 3,000-row list, and the
//! frame that draws the change must do work bounded by the label's depth,
//! not by the number of rows.
//!
//! What is real here: the shadow tree is built and changed through the shim's
//! own C ABI (the renderer ops an IsoNim application calls), and every frame
//! is a real GPUI draw of `NimRootView` in a test-platform window: plan
//! build, element construction, Taffy layout, prepaint and paint. The one
//! stand-in is the test platform itself, which draws into no surface; it is
//! not a mock of anything this probe measures, because everything counted
//! (`gpui_frame_work`) happens before a surface would be touched.
//!
//! The probe reads only the C ABI (`gpui_frame_work`, `gpui_frame_ns`) so that
//! the same file runs, unchanged, against the shim as it was before the plan
//! was made incremental (see `tools/frame-work-probe-control.sh`). There,
//! with the counters added to the old full walk, every count scales with the
//! rows and the probe fails: that is its negative control.
//!
//! The frame times it prints are wall-clock times of the mutation plus the
//! frame, on whatever machine runs it, in the profile it was built in; they
//! are recorded with the commit they were measured at, never asserted.

#![cfg(feature = "gpui-backend")]

use std::ffi::CString;

use gpui::TestAppContext;
use gpui_nim_shim::gpui_app::NimRootView;
use gpui_nim_shim::frame_stats::{
    gpui_frame_count, gpui_frame_ns, gpui_frame_stats_reset, gpui_frame_work,
};
use gpui_nim_shim::{
    gpui_append_child, gpui_create_element, gpui_create_text_node, gpui_reset_tree,
    gpui_set_root_element, gpui_set_style, gpui_set_text_content, GpuiElement,
};
use serial_test::serial;

fn run_gpui_test(name: &'static str, f: fn(&mut TestAppContext)) {
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
}

fn el(tag: &str) -> *mut GpuiElement {
    let tag = CString::new(tag).unwrap();
    gpui_create_element(tag.as_ptr())
}

fn style(node: *mut GpuiElement, prop: &str, value: &str) {
    let p = CString::new(prop).unwrap();
    let v = CString::new(value).unwrap();
    gpui_set_style(node, p.as_ptr(), v.as_ptr());
}

fn text(node: *mut GpuiElement, value: &str) {
    let v = CString::new(value).unwrap();
    gpui_set_text_content(node, v.as_ptr());
}

/// The list: `root > list > rows × (row > label > text)`. A row is a flex
/// row of a fixed height that does not shrink, holding one label, which is
/// what a virtualised list's rows look like. Returns the text nodes.
fn build_list(rows: usize) -> Vec<*mut GpuiElement> {
    gpui_reset_tree();
    let root = el("root");
    style(root, "display", "flex");
    style(root, "flex-direction", "column");
    style(root, "width", "800px");
    style(root, "height", "600px");
    let list = el("div");
    style(list, "display", "flex");
    style(list, "flex-direction", "column");
    style(list, "width", "800px");
    style(list, "height", "600px");
    style(list, "overflow", "hidden");
    gpui_append_child(root, list);
    let mut texts = Vec::with_capacity(rows);
    for i in 0..rows {
        let row = el("div");
        style(row, "display", "flex");
        style(row, "flex-direction", "row");
        style(row, "width", "800px");
        style(row, "height", "20px");
        style(row, "flex-shrink", "0");
        let label = el("span");
        let t = gpui_create_text_node(CString::new(format!("row {i}")).unwrap().as_ptr());
        gpui_append_child(label, t);
        gpui_append_child(row, label);
        gpui_append_child(list, row);
        texts.push(t);
    }
    gpui_set_root_element(root);
    texts
}

/// The last frame's work: nodes visited, plans rebuilt, elements built,
/// subtrees reused, subtrees rendered.
fn last_work() -> [u64; 5] {
    let n = gpui_frame_count();
    assert!(n > 0, "a frame was drawn");
    let mut out = [0u64; 5];
    assert_eq!(gpui_frame_work(n - 1, out.as_mut_ptr()), 1);
    out
}

/// Draw one frame of `view`: the root is told to render, as the shim's
/// repaint poller tells it after any change.
fn frame(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<NimRootView>) {
    cx.update(|_window, app| view.update(app, |_, cx| cx.notify()));
    cx.run_until_parked();
}

/// The depth of a row's text node below the root: root, list, row, label,
/// text. A frame that redraws one changed label may visit and rebuild these
/// five plan nodes; anything proportional to the rows is a failure.
const CHANGE_DEPTH: u64 = 5;
/// What the bound allows beyond the change's own path.
const SLACK: u64 = 2;

struct Measured {
    idle: [u64; 5],
    change: [u64; 5],
    change_ns: Vec<u64>,
    render_ns: Vec<u64>,
}

fn measure(cx: &mut TestAppContext, rows: usize, changes: usize) -> Measured {
    let texts = build_list(rows);
    gpui_frame_stats_reset();
    let (view, vcx) = cx.add_window_view(|_window, _cx| NimRootView::new());
    vcx.run_until_parked();
    // The first frame builds everything; a second settles any state GPUI
    // creates lazily on the first.
    frame(vcx, &view);

    frame(vcx, &view);
    let idle = last_work();

    let mut change = [0u64; 5];
    let mut change_ns = Vec::new();
    let mut render_ns = Vec::new();
    for k in 0..changes {
        let target = texts[(rows / 2 + k * 7) % rows];
        let started = std::time::Instant::now();
        text(target, &format!("changed {k}"));
        frame(vcx, &view);
        change_ns.push(started.elapsed().as_nanos() as u64);
        render_ns.push(gpui_frame_ns(gpui_frame_count() - 1));
        let w = last_work();
        for i in 0..5 {
            change[i] = change[i].max(w[i]);
        }
    }
    Measured { idle, change, change_ns, render_ns }
}

fn median(v: &mut Vec<u64>) -> u64 {
    v.sort_unstable();
    v[v.len() / 2]
}

#[test]
#[serial]
fn one_label_in_a_3000_row_list_costs_its_depth_not_the_rows() {
    run_gpui_test("frame_work_probe", |cx| {
        let small = measure(cx, 300, 5);
        let mut large = measure(cx, 3000, 21);

        let names = ["nodes visited", "plans rebuilt", "elements built", "subtrees reused", "subtrees rendered"];
        for (label, m) in [("300 rows", &small), ("3000 rows", &large)] {
            eprintln!("probe {label}: idle frame {:?}", names.iter().zip(m.idle).collect::<Vec<_>>());
            eprintln!("probe {label}: change frame (max) {:?}", names.iter().zip(m.change).collect::<Vec<_>>());
        }
        eprintln!(
            "probe 3000 rows: change+frame median {} us, render path median {} us (over {} changes)",
            median(&mut large.change_ns) / 1000,
            median(&mut large.render_ns) / 1000,
            large.change_ns.len()
        );

        // An idle frame rebuilds no plan and reads no shadow node.
        for m in [&small, &large] {
            assert_eq!(m.idle[0], 0, "an idle frame visited {} shadow nodes", m.idle[0]);
            assert_eq!(m.idle[1], 0, "an idle frame rebuilt {} plans", m.idle[1]);
        }
        // A one-label frame is bounded by the label's depth...
        for m in [&small, &large] {
            assert!(
                m.change[0] <= CHANGE_DEPTH + SLACK,
                "one label visited {} shadow nodes (bound {})",
                m.change[0],
                CHANGE_DEPTH + SLACK
            );
            assert!(
                m.change[1] <= CHANGE_DEPTH + SLACK,
                "one label rebuilt {} plans (bound {})",
                m.change[1],
                CHANGE_DEPTH + SLACK
            );
        }
        // The GPUI elements built for it are the changed path too: every
        // unchanged row is replayed from a cached view.
        for m in [&small, &large] {
            assert!(
                m.change[2] <= CHANGE_DEPTH + SLACK,
                "one label built {} GPUI elements (bound {})",
                m.change[2],
                CHANGE_DEPTH + SLACK
            );
        }
        // ...and so does not grow with the rows.
        assert_eq!(small.change[0], large.change[0], "nodes visited grew with the rows");
        assert_eq!(small.change[1], large.change[1], "plans rebuilt grew with the rows");
        assert_eq!(small.change[2], large.change[2], "elements built grew with the rows");
    });
}
