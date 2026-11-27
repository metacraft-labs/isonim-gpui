//! Cached subtree views change how much is drawn again, never what is drawn.
//!
//! Each case draws one shadow tree twice in a real GPUI test-platform window:
//! once with every eligible subtree behind a cached view (the boundary
//! threshold at one node) and once with no cached view at all. Every element
//! carries `data-layout-probe`, so the comparison is over where GPUI laid
//! out every element, read back through the same store a host reads
//! (`layout_bounds`), and over what every cut text drew (`cut_text`). The
//! trees are then changed through the C ABI, and the comparison is repeated
//! on the frames that draw the change, which are the frames in which cached
//! views are replayed.
//!
//! The one stand-in is the test platform, which draws into no surface. It
//! lays out, prepaints and paints exactly as a window does, and both sides
//! of every comparison run on it, so it cannot make one side agree with the
//! other.

#![cfg(feature = "gpui-backend")]

use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::atomic::{AtomicU32, Ordering};

use gpui::TestAppContext;
use gpui_nim_shim::cached_views::{set_boundary_min_nodes, DEFAULT_BOUNDARY_MIN_NODES};
use gpui_nim_shim::gpui_app::NimRootView;
use gpui_nim_shim::frame_stats::{gpui_frame_count, gpui_frame_stats_reset, gpui_frame_work};
use gpui_nim_shim::layout_bounds::{bounds_of, gpui_layout_bounds_reset, LaidOut};
use gpui_nim_shim::{
    gpui_add_event_listener, gpui_append_child, gpui_create_element, gpui_create_text_node,
    gpui_node_id, gpui_remove_child, gpui_reset_tree, gpui_set_attribute, gpui_set_root_element,
    gpui_set_style, gpui_set_text_content, GpuiElement,
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
    set_boundary_min_nodes(DEFAULT_BOUNDARY_MIN_NODES);
}

/// Every element made through `el`, so a comparison can read them all.
static MADE: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());

fn made() -> Vec<u64> {
    MADE.lock().unwrap().clone()
}

fn el(tag: &str, styles: &[(&str, &str)]) -> *mut GpuiElement {
    let tag = CString::new(tag).unwrap();
    let e = gpui_create_element(tag.as_ptr());
    for (k, v) in styles {
        style(e, k, v);
    }
    attr(e, "data-layout-probe", "");
    MADE.lock().unwrap().push(gpui_node_id(e));
    e
}

fn style(node: *mut GpuiElement, prop: &str, value: &str) {
    let p = CString::new(prop).unwrap();
    let v = CString::new(value).unwrap();
    gpui_set_style(node, p.as_ptr(), v.as_ptr());
}

fn attr(node: *mut GpuiElement, name: &str, value: &str) {
    let n = CString::new(name).unwrap();
    let v = CString::new(value).unwrap();
    gpui_set_attribute(node, n.as_ptr(), v.as_ptr());
}

fn text_node(parent: *mut GpuiElement, value: &str) -> *mut GpuiElement {
    let v = CString::new(value).unwrap();
    let t = gpui_create_text_node(v.as_ptr());
    gpui_append_child(parent, t);
    t
}

fn set_text(node: *mut GpuiElement, value: &str) {
    let v = CString::new(value).unwrap();
    gpui_set_text_content(node, v.as_ptr());
}

/// The handles a case changes after its first frame.
struct Scene {
    texts: Vec<*mut GpuiElement>,
    rows: Vec<*mut GpuiElement>,
    list: *mut GpuiElement,
}

/// A window's worth of the shapes the shim draws: fixed and percentage
/// sizes, block and flex containers both ways, a stretching column, rows
/// that shrink and rows that do not, margins and borders, an absolute
/// overlay, an inline formatting context, text that is cut with an
/// ellipsis, and a fixed-height list of rows.
fn build_scene() -> Scene {
    gpui_reset_tree();
    MADE.lock().unwrap().clear();
    let root = el("root", &[("display", "flex"), ("flex-direction", "column"), ("width", "640px"), ("height", "480px")]);
    // A header: a flex row, its own children content-sized.
    let header = el("div", &[("display", "flex"), ("flex-direction", "row"), ("width", "100%"), ("height", "32px"), ("flex-shrink", "0"), ("padding", "4px"), ("gap", "6px")]);
    gpui_append_child(root, header);
    for w in ["60px", "90px"] {
        let b = el("div", &[("display", "flex"), ("width", w), ("height", "24px"), ("border-width", "1px"), ("border-color", "#888888")]);
        let s = el("span", &[]);
        text_node(s, "btn");
        gpui_append_child(b, s);
        gpui_append_child(header, b);
    }
    // A body: two panes side by side, the second one grows.
    let body = el("div", &[("display", "flex"), ("flex-direction", "row"), ("flex-grow", "1"), ("width", "100%")]);
    gpui_append_child(root, body);
    let left = el("div", &[("display", "flex"), ("flex-direction", "column"), ("width", "200px"), ("flex-shrink", "0"), ("overflow", "hidden")]);
    gpui_append_child(body, left);
    let right = el("div", &[("display", "flex"), ("flex-direction", "column"), ("flex-grow", "1"), ("overflow", "hidden"), ("margin-left", "3px")]);
    gpui_append_child(body, right);

    // The left pane: a fixed-height list of rows that do not shrink, each a
    // row of two labels, one of them cut.
    let list = el("div", &[("display", "flex"), ("flex-direction", "column"), ("width", "100%"), ("height", "300px"), ("overflow", "hidden")]);
    gpui_append_child(left, list);
    let mut texts = Vec::new();
    let mut rows = Vec::new();
    for i in 0..12 {
        let row = el("div", &[("display", "flex"), ("flex-direction", "row"), ("height", "18px"), ("flex-shrink", "0"), ("margin-top", if i % 3 == 0 { "2px" } else { "0px" })]);
        let a = el("span", &[("width", "40px"), ("flex-shrink", "0")]);
        texts.push(text_node(a, &format!("{i}")));
        let b = el("span", &[("white-space", "nowrap"), ("overflow", "hidden"), ("text-overflow", "ellipsis"), ("flex-grow", "1")]);
        texts.push(text_node(b, &format!("a long value that will not fit in the pane, number {i}")));
        gpui_append_child(row, a);
        gpui_append_child(row, b);
        gpui_append_child(list, row);
        rows.push(row);
    }
    // A content-sized block below it, with block children (whose margins a
    // block parent may collapse).
    let block = el("div", &[("padding", "2px")]);
    gpui_append_child(left, block);
    for i in 0..3 {
        let p = el("p", &[("margin-top", "4px"), ("margin-bottom", "4px")]);
        texts.push(text_node(p, &format!("paragraph {i}")));
        gpui_append_child(block, p);
    }

    // The right pane: rows with an inline formatting context, a stretching
    // column of boxes, and an absolute overlay.
    for i in 0..4 {
        let line = el("div", &[("display", "flex"), ("flex-direction", "row"), ("white-space", "nowrap"), ("height", "20px"), ("flex-shrink", "0")]);
        attr(line, "data-inline-text-runs", "");
        for (j, c) in ["#ff0000", "#00aa00", "#0000ff"].iter().enumerate() {
            let s = el("span", &[("color", c)]);
            texts.push(text_node(s, &format!("seg{i}{j} ")));
            gpui_append_child(line, s);
        }
        gpui_append_child(right, line);
    }
    let column = el("div", &[("display", "flex"), ("flex-direction", "column"), ("height", "120px"), ("flex-shrink", "0")]);
    gpui_append_child(right, column);
    for h in ["30px", "40px"] {
        let b = el("div", &[("display", "flex"), ("height", h), ("flex-shrink", "0"), ("background-color", "#eeeeee"), ("padding", "3px")]);
        let s = el("span", &[]);
        texts.push(text_node(s, h));
        gpui_append_child(b, s);
        gpui_append_child(column, b);
    }
    let overlay = el("div", &[("display", "flex"), ("position", "absolute"), ("left", "50px"), ("top", "60px"), ("width", "150px"), ("height", "80px"), ("background-color", "#333333")]);
    let s = el("span", &[("color", "#ffffff")]);
    texts.push(text_node(s, "overlay"));
    gpui_append_child(overlay, s);
    gpui_append_child(right, overlay);

    gpui_set_root_element(root);
    Scene { texts, rows, list }
}

/// Everything the window reports about the scene's layout and text.
#[derive(Debug, PartialEq)]
struct Observed {
    bounds: BTreeMap<u64, (i64, i64, i64, i64)>,
    drawn: BTreeMap<u64, String>,
}

fn observe() -> Observed {
    let q = |v: f32| (v * 64.0).round() as i64;
    let mut bounds = BTreeMap::new();
    let mut drawn = BTreeMap::new();
    for id in made() {
        if let Some(LaidOut { x, y, w, h }) = bounds_of(id) {
            bounds.insert(id, (q(x), q(y), q(w), q(h)));
        }
        if let Some(t) = gpui_nim_shim::cut_text::drawn_text_of(id) {
            drawn.insert(id, t);
        }
    }
    Observed { bounds, drawn }
}

fn frame(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<NimRootView>) {
    cx.update(|_window, app| view.update(app, |_, cx| cx.notify()));
    cx.run_until_parked();
}

/// Draw the scene with `min_nodes` as the boundary threshold, apply
/// `changes` one frame each, and observe after every frame. The node ids
/// differ between two builds, so ids are mapped to creation order.
fn run(cx: &mut TestAppContext, min_nodes: usize, changes: &[fn(&Scene)]) -> (Vec<Observed>, u64) {
    set_boundary_min_nodes(min_nodes);
    let scene = build_scene();
    gpui_frame_stats_reset();
    gpui_layout_bounds_reset();
    gpui_nim_shim::cut_text::reset();
    let (view, vcx) = cx.add_window_view(|_window, _cx| NimRootView::new());
    vcx.run_until_parked();
    frame(vcx, &view);
    let order: BTreeMap<u64, u64> = made().iter().enumerate().map(|(i, id)| (*id, i as u64)).collect();
    let renumber = |o: Observed| Observed {
        bounds: o.bounds.into_iter().map(|(k, v)| (order[&k], v)).collect(),
        drawn: o.drawn.into_iter().map(|(k, v)| (order[&k], v)).collect(),
    };
    let mut seen = vec![renumber(observe())];
    for change in changes {
        change(&scene);
        frame(vcx, &view);
        seen.push(renumber(observe()));
    }
    // How many subtrees the frames replayed, so a case can tell that the
    // cached side really cached.
    let mut reused = 0;
    for i in 0..gpui_frame_count() {
        let mut w = [0u64; 5];
        gpui_frame_work(i, w.as_mut_ptr());
        reused += w[3];
    }
    (seen, reused)
}

fn changes() -> Vec<fn(&Scene)> {
    vec![
        // One label in the list.
        |s| set_text(s.texts[6], "77"),
        // A cut text grows.
        |s| set_text(s.texts[9], "a still longer value that will certainly not fit in the pane"),
        // A row's style changes its height.
        |s| style(s.rows[3], "height", "26px"),
        // A row leaves the list; the rows after it move up.
        |s| gpui_remove_child(s.list, s.rows[1]),
        // A row comes back at the end.
        |s| gpui_append_child(s.list, s.rows[1]),
        // A paragraph's text wraps differently.
        |s| set_text(s.texts[24], "a paragraph that is much longer than the others and wraps"),
        // An inline segment changes colour and text.
        |s| {
            set_text(s.texts[28], "changed ");
        },
        // Nothing changes at all.
        |_s| {},
    ]
}

#[test]
#[serial]
fn cached_views_lay_out_and_cut_exactly_as_plain_elements() {
    run_gpui_test("cached_views_equivalence", |cx| {
        let (plain, plain_reused) = run(cx, usize::MAX, &changes());
        let (cached, cached_reused) = run(cx, 1, &changes());
        assert_eq!(plain_reused, 0, "control: with no boundary nothing is replayed");
        assert!(cached_reused > 0, "the cached side replayed {cached_reused} subtrees");
        assert_eq!(plain.len(), cached.len());
        for (i, (p, c)) in plain.iter().zip(&cached).enumerate() {
            assert!(p.bounds.len() > 40, "frame {i}: the probes were laid out ({})", p.bounds.len());
            assert!(!p.drawn.is_empty(), "frame {i}: the cut texts were drawn");
            for (k, v) in &p.bounds {
                assert_eq!(Some(v), c.bounds.get(k), "frame {i}: element #{k} (bounds in 1/64 px)");
            }
            assert_eq!(p, c, "frame {i}");
        }
    });
}

/// A host that resets the bounds store reads every element again on the
/// next frame, although nothing changed and every subtree is replayed.
#[test]
#[serial]
fn a_reset_store_is_filled_again_by_the_next_frame() {
    run_gpui_test("cached_views_reset", |cx| {
        set_boundary_min_nodes(1);
        build_scene();
        let (view, vcx) = cx.add_window_view(|_window, _cx| NimRootView::new());
        vcx.run_until_parked();
        frame(vcx, &view);
        let before = observe();
        assert!(before.bounds.len() > 40);
        gpui_layout_bounds_reset();
        assert!(observe().bounds.is_empty(), "control: the reset emptied the store");
        frame(vcx, &view);
        assert_eq!(observe().bounds, before.bounds);
    });
}

static CLICKS: AtomicU32 = AtomicU32::new(0);
extern "C" fn on_click(_p: *const gpui_nim_shim::input::GpuiEventPayload) {
    CLICKS.fetch_add(1, Ordering::SeqCst);
}

/// A row replayed from a cached view still takes the pointer: its hitbox and
/// its listeners are replayed with it.
#[test]
#[serial]
fn a_replayed_row_still_takes_a_click() {
    run_gpui_test("cached_views_click", |cx| {
        set_boundary_min_nodes(1);
        let scene = build_scene();
        let target = scene.rows[5];
        let click = CString::new("click").unwrap();
        gpui_add_event_listener(target, click.as_ptr(), on_click);
        let (view, vcx) = cx.add_window_view(|_window, _cx| NimRootView::new());
        vcx.run_until_parked();
        frame(vcx, &view);
        // Change a different row, so the frame replays this one.
        set_text(scene.texts[0], "changed");
        gpui_frame_stats_reset();
        frame(vcx, &view);
        let mut w = [0u64; 5];
        assert_eq!(gpui_frame_work(0, w.as_mut_ptr()), 1);
        assert!(w[3] > 0, "the frame replayed cached subtrees");
        let b = bounds_of(gpui_node_id(target)).expect("the row was laid out");
        let at = gpui::point(gpui::px(b.x + 10.0), gpui::px(b.y + b.h / 2.0));
        CLICKS.store(0, Ordering::SeqCst);
        vcx.simulate_click(at, gpui::Modifiers::default());
        vcx.run_until_parked();
        assert_eq!(CLICKS.load(Ordering::SeqCst), 1, "the replayed row took the click");
        // Control: a click beside the list reaches nothing.
        let away = gpui::point(gpui::px(500.0), gpui::px(470.0));
        vcx.simulate_click(away, gpui::Modifiers::default());
        vcx.run_until_parked();
        assert_eq!(CLICKS.load(Ordering::SeqCst), 1);
    });
}
