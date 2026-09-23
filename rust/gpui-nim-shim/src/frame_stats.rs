//! Frame timing, recorded by the render path and read over the C ABI.
//!
//! Two series, both in nanoseconds:
//!
//! * **render-path time per frame** — from the start of `NimRootView::render`'s
//!   plan build to the GPUI element tree being returned. This is the part of a
//!   frame this shim and its host control (shadow-tree walk, render plan,
//!   element conversion). GPUI's own layout and paint run AFTER `render`
//!   returns and are NOT in this number; a consumer reporting it as "the frame"
//!   would overstate what was measured.
//! * **key-to-frame latency** — from a key being delivered to the focused
//!   element (`input::deliver_key_to_focus`, i.e. after the host's handler ran
//!   and mutated the tree) to the END of the next render. It includes the
//!   repaint poller's wait, so it is what a keystroke costs before paint.
//!
//! Feature-less on purpose, like `input`: the exported symbol set must not
//! depend on the cdylib's features (see `gpui_headless_unavailable.rs`).
//!
//! Bounded: each series keeps its first `CAP` samples, so a window left open
//! for hours cannot grow without limit. `gpui_frame_stats_reset` clears both.

use std::sync::Mutex;
use std::time::Instant;

const CAP: usize = 1_000_000;

static FRAMES: Mutex<Vec<u64>> = Mutex::new(Vec::new());
static LATENCIES: Mutex<Vec<u64>> = Mutex::new(Vec::new());
static PENDING_KEY: Mutex<Option<Instant>> = Mutex::new(None);

fn push(series: &Mutex<Vec<u64>>, ns: u64) {
    let mut v = series.lock().unwrap_or_else(|p| p.into_inner());
    if v.len() < CAP {
        v.push(ns);
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

/// A render finished, having taken `render_ns` of render-path time.
pub fn complete_frame(render_ns: u64) {
    push(&FRAMES, render_ns);
    let pending = PENDING_KEY
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take();
    if let Some(t) = pending {
        push(&LATENCIES, t.elapsed().as_nanos() as u64);
    }
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

#[no_mangle]
pub extern "C" fn gpui_frame_stats_reset() {
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
        complete_frame(1_000);
        assert_eq!(gpui_frame_count(), 1);
        assert_eq!(gpui_frame_ns(0), 1_000);
        assert_eq!(gpui_key_latency_count(), 0);

        // Two keys before one frame are ONE latency sample, measured from the
        // FIRST key.
        note_key();
        std::thread::sleep(std::time::Duration::from_millis(5));
        note_key();
        complete_frame(2_000);
        assert_eq!(gpui_frame_count(), 2);
        assert_eq!(gpui_key_latency_count(), 1);
        assert!(gpui_key_latency_ns(0) >= 5_000_000);

        // Past the end reads 0 rather than panicking across the ABI.
        assert_eq!(gpui_frame_ns(99), 0);
        assert_eq!(gpui_key_latency_ns(99), 0);

        gpui_frame_stats_reset();
        assert_eq!(gpui_frame_count(), 0);
        assert_eq!(gpui_key_latency_count(), 0);
    }
}
