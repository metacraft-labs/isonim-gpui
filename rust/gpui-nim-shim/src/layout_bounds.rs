//! Laid-out bounds, read back: where GPUI actually put an element.
//!
//! The shadow tree says what an element IS (its tag, attributes and styles,
//! and the render plan built from them); nothing in it says where the window
//! put it, because the layout is GPUI's (Taffy's), computed after the render
//! plan has been handed over. A host that has to compare a window's layout
//! with another renderer's -- CodeTracer's GPUI captures against the web
//! captures of the same story -- needs the second fact, read off the layout
//! GPUI computed, not re-derived in the host (a second derivation of a layout
//! agrees with itself, not with the window).
//!
//! An element OPTS IN with the attribute `data-layout-probe` (any value). Its
//! `Div` is then given one more child, an absolutely positioned, full-size
//! `canvas` that paints nothing and records its bounds during prepaint
//! (`gpui_app::with_layout_probe`). An absolute child takes no part in its
//! parent's flex layout, so the probe does not move what it measures. The
//! canvas is laid out over the parent's PADDING box -- the containing block
//! of an absolute child (CSS 2.1 §10.1, which Taffy follows:
//! <https://www.w3.org/TR/CSS21/visudet.html#containing-block-details>) --
//! and the element's own border widths are added back on each side
//! (`border_box_of`), so what is recorded is its BORDER box, in window
//! coordinates, logical pixels: the box a browser's `getBoundingClientRect`
//! reports (CSSOM View §6,
//! <https://www.w3.org/TR/cssom-view-1/#dom-element-getboundingclientrect>).
//! Until 2026-11-10 the padding box itself was recorded, and every bordered
//! element read its border width smaller on each side than the browser's:
//! a list of rows with a 1px hairline under each read every row 1px short
//! although the rows stood exactly as far apart as the browser's.
//!
//! Only elements that opt in are recorded: a probe on every node would add a
//! child to every node of every application.
//!
//! Feature-less on purpose, like `input` and `frame_stats`: the store and its
//! C ABI exist in every build, so the cdylib's exported symbol set does not
//! depend on its features (see `gpui_headless_unavailable.rs`). Without the
//! GPUI backend nothing lays anything out, and every read answers "not laid
//! out".

use std::collections::HashMap;
use std::sync::Mutex;

use crate::{handle_to_node_id, GpuiElement};

/// The attribute an element opts in with.
pub const LAYOUT_PROBE_ATTRIBUTE: &str = "data-layout-probe";

/// One element's bounds: origin and size, window coordinates, logical px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LaidOut {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

// Written from prepaint and read over the C ABI; GPUI prepaints on the main
// thread, which is also where a host's tick callback reads, but a Mutex
// keeps the store sound if a reader is elsewhere.
static BOUNDS: Mutex<Option<HashMap<u64, LaidOut>>> = Mutex::new(None);

/// Whether a plan node with these attributes asked for its bounds.
pub fn wants_probe(attributes: &std::collections::BTreeMap<String, String>) -> bool {
    attributes.contains_key(LAYOUT_PROBE_ATTRIBUTE)
}

/// The border box of an element whose padding box is `padding` and whose
/// border is `widths` wide on each side (top, right, bottom, left).
pub fn border_box_of(padding: LaidOut, widths: [f32; 4]) -> LaidOut {
    let [top, right, bottom, left] = widths;
    LaidOut {
        x: padding.x - left,
        y: padding.y - top,
        w: padding.w + left + right,
        h: padding.h + top + bottom,
    }
}

/// The window laid `node_id` out at these bounds (its latest frame wins).
pub fn record(node_id: u64, bounds: LaidOut) {
    let mut store = BOUNDS.lock().unwrap_or_else(|p| p.into_inner());
    store.get_or_insert_with(HashMap::new).insert(node_id, bounds);
}

/// The latest bounds recorded for `node_id`, if any frame laid it out.
pub fn bounds_of(node_id: u64) -> Option<LaidOut> {
    let store = BOUNDS.lock().unwrap_or_else(|p| p.into_inner());
    store.as_ref().and_then(|m| m.get(&node_id).copied())
}

/// Forget every recorded bounds (a new tree, or a new measurement).
pub fn reset() {
    let mut store = BOUNDS.lock().unwrap_or_else(|p| p.into_inner());
    *store = None;
}

/// The bounds the window last laid `node` out at, written to `out[0..4]` as
/// x, y, width, height (window coordinates, logical pixels). Answers 1 when
/// it wrote them and 0 when it did not: a null handle, a null `out`, an
/// element without `data-layout-probe`, or one no frame has laid out yet
/// (including every element of a build without the GPUI backend).
///
/// # Safety
/// `out`, when not null, must point at four writable `f64`s.
#[no_mangle]
pub extern "C" fn gpui_layout_bounds(node: *mut GpuiElement, out: *mut f64) -> u8 {
    if out.is_null() {
        return 0;
    }
    let id = unsafe { handle_to_node_id(node) };
    if id.is_null() {
        return 0;
    }
    match bounds_of(id.0) {
        Some(b) => {
            let values = [b.x as f64, b.y as f64, b.w as f64, b.h as f64];
            for (i, v) in values.iter().enumerate() {
                unsafe { *out.add(i) = *v };
            }
            1
        }
        None => 0,
    }
}

/// Forget every bounds recorded so far, so the next read answers only what a
/// later frame laid out.
#[no_mangle]
pub extern "C" fn gpui_layout_bounds_reset() {
    reset();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    // The store is process-global; these run in one test so they cannot
    // interleave with each other.
    #[test]
    fn a_border_box_adds_each_side_s_border_back() {
        let padding = LaidOut { x: 10.0, y: 20.0, w: 100.0, h: 50.0 };
        // No border: the padding box is the border box.
        assert_eq!(border_box_of(padding, [0.0; 4]), padding);
        // A 1px hairline under a row: one pixel taller, nowhere else moved.
        assert_eq!(
            border_box_of(padding, [0.0, 0.0, 1.0, 0.0]),
            LaidOut { x: 10.0, y: 20.0, w: 100.0, h: 51.0 }
        );
        // Every side different.
        assert_eq!(
            border_box_of(padding, [1.0, 2.0, 3.0, 4.0]),
            LaidOut { x: 6.0, y: 19.0, w: 106.0, h: 54.0 }
        );
    }

    #[test]
    fn records_reads_and_forgets() {
        reset();
        assert_eq!(bounds_of(7), None);
        record(7, LaidOut { x: 1.0, y: 2.0, w: 30.0, h: 40.0 });
        assert_eq!(bounds_of(7), Some(LaidOut { x: 1.0, y: 2.0, w: 30.0, h: 40.0 }));
        // The latest frame wins.
        record(7, LaidOut { x: 5.0, y: 2.0, w: 30.0, h: 40.0 });
        assert_eq!(bounds_of(7).map(|b| b.x), Some(5.0));
        assert_eq!(bounds_of(8), None);
        reset();
        assert_eq!(bounds_of(7), None);

        // The C ABI refuses what it cannot answer instead of writing junk.
        let mut out = [-1.0f64; 4];
        assert_eq!(gpui_layout_bounds(std::ptr::null_mut(), out.as_mut_ptr()), 0);
        assert_eq!(out, [-1.0; 4]);

        // A real element of the shadow tree: unread until laid out, then
        // read back through its handle exactly as recorded.
        let el = crate::gpui_create_element(c"div".as_ptr());
        assert_eq!(gpui_layout_bounds(el, out.as_mut_ptr()), 0);
        assert_eq!(gpui_layout_bounds(el, std::ptr::null_mut()), 0);
        let id = crate::gpui_node_id(el);
        record(id, LaidOut { x: 10.0, y: 20.0, w: 300.5, h: 40.0 });
        assert_eq!(gpui_layout_bounds(el, out.as_mut_ptr()), 1);
        assert_eq!(out, [10.0, 20.0, 300.5, 40.0]);
        gpui_layout_bounds_reset();
        assert_eq!(gpui_layout_bounds(el, out.as_mut_ptr()), 0);
        crate::gpui_destroy_element(el);

        let mut attrs = BTreeMap::new();
        assert!(!wants_probe(&attrs));
        attrs.insert(LAYOUT_PROBE_ATTRIBUTE.to_string(), String::new());
        assert!(wants_probe(&attrs));
    }
}
