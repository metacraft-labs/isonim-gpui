//! Frame timing and frame WORK, recorded by the render path and read over
//! the C ABI.
//!
//! Two timing series, both in nanoseconds:
//!
//! * **render-path time per frame** — the shim's own share of a frame: the
//!   plan build and element construction in `NimRootView::render`, plus the
//!   element construction of every cached subtree view GPUI asked to render
//!   again in that frame (`gpui_app::NimSubtreeView`). This is the part of a
//!   frame this shim and its host control. GPUI's own layout and paint are
//!   NOT in this number; a consumer reporting it as "the frame" would
//!   overstate what was measured.
//! * **key-to-frame latency** — from a key being delivered to the focused
//!   element (`input::deliver_key_to_focus`, i.e. after the host's handler ran
//!   and mutated the tree) to the end of the next frame's render path. It
//!   includes the repaint poller's wait, so it is what a keystroke costs
//!   before paint.
//!
//! And one series of COUNTS per frame ([`FrameWork`]), which is what makes
//! "per-frame work proportional to what changed" checkable rather than a
//! claim: the shadow nodes the plan build visited, the plans it rebuilt, the
//! GPUI elements constructed, and the cached subtrees reused and rendered.
//!
//! A frame is opened by `begin_frame` at the top of `NimRootView::render`
//! and closed by `complete_frame`, which runs from the prepaint of a marker
//! element drawn after everything else, because GPUI renders a cached view
//! that must be drawn again during the frame's layout and prepaint, after
//! the root's `render` has returned.
//!
//! Feature-less on purpose, like `input`: the exported symbol set must not
//! depend on the cdylib's features (see `gpui_headless_unavailable.rs`).
//!
//! Bounded: each series keeps its first `CAP` samples, so a window left open
//! for hours cannot grow without limit. `gpui_frame_stats_reset` clears them.

use std::cell::Cell;
use std::sync::Mutex;
use std::time::Instant;

const CAP: usize = 1_000_000;

static FRAMES: Mutex<Vec<u64>> = Mutex::new(Vec::new());
static LATENCIES: Mutex<Vec<u64>> = Mutex::new(Vec::new());
static WORK: Mutex<Vec<FrameWork>> = Mutex::new(Vec::new());
static PENDING_KEY: Mutex<Option<Instant>> = Mutex::new(None);

/// Frames drawn since the process started: one per `complete_frame` that
/// closed a frame. MONOTONIC, unlike `gpui_frame_count` (which
/// `gpui_frame_stats_reset` clears and `CAP` bounds), so a host can tell "a
/// frame was drawn after this moment" by comparing two reads.
static FRAMES_DRAWN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A host's "a frame was drawn" callback (`gpui_on_frame_drawn`).
pub type FrameDrawnCallback = extern "C" fn();

static FRAME_DRAWN: Mutex<Option<FrameDrawnCallback>> = Mutex::new(None);

/// The epoch cached subtree views are drawn in. A view drawn at an older
/// epoch is drawn again rather than replayed: see `cached_views`.
static CACHED_SUBTREE_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The current epoch of cached subtree views.
pub fn cached_subtree_epoch() -> u64 {
    CACHED_SUBTREE_EPOCH.load(std::sync::atomic::Ordering::Relaxed)
}

/// Make the next frame draw every cached subtree again. Called when a store
/// written during prepaint (`layout_bounds`, `cut_text`) is reset, since a
/// replayed subtree is not prepainted and would not fill it again.
pub fn invalidate_cached_subtrees() {
    CACHED_SUBTREE_EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// What one frame's render path did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameWork {
    /// Shadow nodes the plan build read because they or something below
    /// them changed (`render_sync::PlanWork::nodes_visited`).
    pub nodes_visited: u64,
    /// Plan nodes built again (`render_sync::PlanWork::plans_rebuilt`).
    pub plans_rebuilt: u64,
    /// GPUI elements constructed from plan nodes, by the root view and by
    /// every cached subtree view rendered again.
    pub elements_built: u64,
    /// Cached subtree views drawn from the previous frame's work: neither
    /// their elements, nor their layout, prepaint or paint were redone.
    pub subtrees_reused: u64,
    /// Cached subtree views rendered (again or for the first time).
    pub subtrees_rendered: u64,
}

impl FrameWork {
    /// The counts in the order `gpui_frame_work` writes them.
    pub fn to_array(self) -> [u64; FRAME_WORK_FIELDS] {
        [
            self.nodes_visited,
            self.plans_rebuilt,
            self.elements_built,
            self.subtrees_reused,
            self.subtrees_rendered,
        ]
    }
}

/// How many counts `gpui_frame_work` writes.
pub const FRAME_WORK_FIELDS: usize = 5;

// The frame being built. Thread-local: a frame is built on one thread (the
// window's main thread, or the headless worker's), and the element count is
// bumped once per element, which is too hot a path for a lock.
thread_local! {
    static OPEN: Cell<bool> = const { Cell::new(false) };
    static RENDER_NS: Cell<u64> = const { Cell::new(0) };
    static CURRENT: Cell<FrameWork> = const { Cell::new(FrameWork {
        nodes_visited: 0,
        plans_rebuilt: 0,
        elements_built: 0,
        subtrees_reused: 0,
        subtrees_rendered: 0,
    }) };
}

fn bump(f: impl FnOnce(&mut FrameWork)) {
    CURRENT.with(|c| {
        let mut w = c.get();
        f(&mut w);
        c.set(w);
    });
}

/// A root render began: start counting a new frame.
pub fn begin_frame() {
    OPEN.with(|o| o.set(true));
    RENDER_NS.with(|r| r.set(0));
    CURRENT.with(|c| c.set(FrameWork::default()));
}

/// Render-path time spent on this frame (the root's, or a subtree view's).
pub fn add_render_ns(ns: u64) {
    RENDER_NS.with(|r| r.set(r.get() + ns));
}

/// The plan build's share of this frame.
pub fn note_plan(work: crate::render_sync::PlanWork) {
    bump(|w| {
        w.nodes_visited += work.nodes_visited;
        w.plans_rebuilt += work.plans_rebuilt;
    });
}

/// One GPUI element was constructed from a plan node.
pub fn note_element() {
    bump(|w| w.elements_built += 1);
}

/// A cached subtree view was kept from the previous frame.
pub fn note_subtree_reused() {
    bump(|w| w.subtrees_reused += 1);
}

/// A cached subtree view rendered.
pub fn note_subtree_rendered() {
    bump(|w| w.subtrees_rendered += 1);
}

/// The frame's work so far (for tests that inspect a frame in progress).
pub fn current_work() -> FrameWork {
    CURRENT.with(|c| c.get())
}

fn push<T>(series: &Mutex<Vec<T>>, value: T) {
    let mut v = series.lock().unwrap_or_else(|p| p.into_inner());
    if v.len() < CAP {
        v.push(value);
    }
}

/// A key reached the focused element. Only the FIRST key since the last frame
/// starts the clock: several keys before one frame are one frame's latency.
pub fn note_key() {
    let mut k = PENDING_KEY.lock().unwrap_or_else(|p| p.into_inner());
    if k.is_none() {
        *k = Some(Instant::now());
    }
}

/// The frame opened by `begin_frame` is drawn: record its render-path time
/// and its work, and count it (`gpui_frames_drawn`). A call with no frame open
/// (a fallback root that drew no plan) records nothing. Answers whether a
/// frame was closed, so the caller can tell the host (`gpui_on_frame_drawn`).
pub fn complete_frame() -> bool {
    if !OPEN.with(|o| o.replace(false)) {
        return false;
    }
    FRAMES_DRAWN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let render_ns = RENDER_NS.with(|r| r.get());
    push(&FRAMES, render_ns);
    push(&WORK, current_work());
    let pending = PENDING_KEY
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take();
    if let Some(t) = pending {
        push(&LATENCIES, t.elapsed().as_nanos() as u64);
    }
    true
}

/// Frames drawn since the process started (`FRAMES_DRAWN`): monotonic, never
/// reset.
#[no_mangle]
pub extern "C" fn gpui_frames_drawn() -> u64 {
    FRAMES_DRAWN.load(std::sync::atomic::Ordering::SeqCst)
}

/// Register (or, with `None`, remove) the host's callback for a drawn frame.
/// The shim calls it once per drawn frame, on the main thread, AFTER that
/// frame's paint (deferred from the frame's end marker, `gpui_app.rs`), so it
/// may read the host's state and the shim's freely; it is never called while
/// nothing is drawn. With none registered a frame costs one lock and a check.
#[no_mangle]
pub extern "C" fn gpui_on_frame_drawn(callback: Option<FrameDrawnCallback>) {
    *FRAME_DRAWN.lock().unwrap_or_else(|p| p.into_inner()) = callback;
}

/// The registered drawn-frame callback, if any.
pub fn frame_drawn_callback() -> Option<FrameDrawnCallback> {
    *FRAME_DRAWN.lock().unwrap_or_else(|p| p.into_inner())
}

/// The `index`-th frame's work, or `None` past the end.
pub fn frame_work(index: u64) -> Option<FrameWork> {
    let v = WORK.lock().unwrap_or_else(|p| p.into_inner());
    v.get(index as usize).copied()
}

fn read(series: &Mutex<Vec<u64>>, index: u64) -> u64 {
    let v = series.lock().unwrap_or_else(|p| p.into_inner());
    v.get(index as usize).copied().unwrap_or(0)
}

fn len(series: &Mutex<Vec<u64>>) -> u64 {
    series.lock().unwrap_or_else(|p| p.into_inner()).len() as u64
}

#[no_mangle]
pub extern "C" fn gpui_frame_count() -> u64 {
    len(&FRAMES)
}

/// The `index`-th frame's render-path time in ns, or 0 past the end.
#[no_mangle]
pub extern "C" fn gpui_frame_ns(index: u64) -> u64 {
    read(&FRAMES, index)
}

#[no_mangle]
pub extern "C" fn gpui_key_latency_count() -> u64 {
    len(&LATENCIES)
}

/// The `index`-th key-to-frame latency in ns, or 0 past the end.
#[no_mangle]
pub extern "C" fn gpui_key_latency_ns(index: u64) -> u64 {
    read(&LATENCIES, index)
}

/// The `index`-th frame's work, written to `out[0..5]`: shadow nodes
/// visited by the plan build, plans rebuilt, GPUI elements built, cached
/// subtrees reused and cached subtrees rendered (`FrameWork`, in that
/// order). Answers 1 when it wrote them and 0 past the end or for a null
/// `out`, which it leaves untouched.
///
/// # Safety
/// `out`, when not null, must point at five writable `u64`s.
#[no_mangle]
pub extern "C" fn gpui_frame_work(index: u64, out: *mut u64) -> u8 {
    if out.is_null() {
        return 0;
    }
    match frame_work(index) {
        Some(w) => {
            for (i, v) in w.to_array().iter().enumerate() {
                unsafe { *out.add(i) = *v };
            }
            1
        }
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn gpui_frame_stats_reset() {
    WORK.lock().unwrap_or_else(|p| p.into_inner()).clear();
    FRAMES.lock().unwrap_or_else(|p| p.into_inner()).clear();
    LATENCIES.lock().unwrap_or_else(|p| p.into_inner()).clear();
    *PENDING_KEY.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    // ONE TEST FUNCTION: the series are process-global, and parallel tests
    // over them would race each other's resets.
    #[test]
    fn frames_and_key_latencies_are_recorded_and_reset() {
        gpui_frame_stats_reset();
        assert_eq!(gpui_frame_count(), 0);
        assert_eq!(gpui_key_latency_count(), 0);

        // A frame with no key before it records a frame and no latency.
        begin_frame();
        add_render_ns(1_000);
        complete_frame();
        assert_eq!(gpui_frame_count(), 1);
        assert_eq!(gpui_frame_ns(0), 1_000);
        assert_eq!(gpui_key_latency_count(), 0);

        // Two keys before one frame are ONE latency sample, measured from the
        // FIRST key. The render-path time is the sum of the frame's parts.
        note_key();
        std::thread::sleep(std::time::Duration::from_millis(5));
        note_key();
        begin_frame();
        add_render_ns(1_500);
        add_render_ns(500);
        complete_frame();
        assert_eq!(gpui_frame_count(), 2);
        assert_eq!(gpui_frame_ns(1), 2_000);
        assert_eq!(gpui_key_latency_count(), 1);
        assert!(gpui_key_latency_ns(0) >= 5_000_000);

        // A completion with no frame open records nothing.
        complete_frame();
        assert_eq!(gpui_frame_count(), 2);

        // The work counts are per frame and read back in ABI order.
        begin_frame();
        note_plan(crate::render_sync::PlanWork { nodes_visited: 3, plans_rebuilt: 2 });
        note_element();
        note_element();
        note_subtree_reused();
        note_subtree_rendered();
        complete_frame();
        let mut out = [0u64; FRAME_WORK_FIELDS];
        assert_eq!(gpui_frame_work(2, out.as_mut_ptr()), 1);
        assert_eq!(out, [3, 2, 2, 1, 1]);
        assert_eq!(gpui_frame_work(0, out.as_mut_ptr()), 1);
        assert_eq!(out, [0; FRAME_WORK_FIELDS]);
        assert_eq!(gpui_frame_work(99, out.as_mut_ptr()), 0);
        assert_eq!(gpui_frame_work(0, std::ptr::null_mut()), 0);

        // The monotonic count moves once per CLOSED frame, not for a
        // completion with no frame open; a reset of the series (at the end)
        // does not move it back.
        let drawn = gpui_frames_drawn();
        begin_frame();
        assert!(complete_frame());
        assert_eq!(gpui_frames_drawn(), drawn + 1);
        assert!(!complete_frame());
        assert_eq!(gpui_frames_drawn(), drawn + 1);

        // The drawn-frame callback is registered, read back and removed.
        extern "C" fn on_drawn() {}
        assert!(frame_drawn_callback().is_none());
        gpui_on_frame_drawn(Some(on_drawn));
        assert!(frame_drawn_callback().map(|f| f as usize) == Some(on_drawn as usize));
        gpui_on_frame_drawn(None);
        assert!(frame_drawn_callback().is_none());

        // Past the end reads 0 rather than panicking across the ABI.
        assert_eq!(gpui_frame_ns(99), 0);
        assert_eq!(gpui_key_latency_ns(99), 0);

        let before_reset = gpui_frames_drawn();
        gpui_frame_stats_reset();
        assert_eq!(gpui_frame_count(), 0);
        assert_eq!(gpui_key_latency_count(), 0);
        assert_eq!(gpui_frame_work(0, out.as_mut_ptr()), 0);
        assert_eq!(gpui_frames_drawn(), before_reset);
    }
}
