//! EMC2-M1 — Rust-side integration test for the dedicated GPUI render
//! thread (Approach 1).
//!
//! Asserts:
//!   * Submitting a render request via ``gpui_render_submit_async``
//!     returns immediately with a non-zero token (the bridge thread
//!     is not blocked on the ~41 ms ``Window::render_to_image`` body).
//!   * Polling via ``gpui_render_try_take`` eventually transitions
//!     from Pending (1) to Ready (0); when Ready, the buffer is
//!     ``width * height * 4`` bytes and contains varied content.
//!   * Two consecutive submits both return non-blocking tokens (the
//!     bridge can pipeline frame N+1 while N-1 finishes).
//!   * Unknown / cancelled tokens behave correctly.

#![cfg(feature = "gpui-headless")]

use std::collections::HashSet;
use std::ffi::CString;
use std::time::{Duration, Instant};

use gpui_nim_shim::gpui_headless::{
    abandon_count_for_tests, gpui_bump_generation, gpui_free_pixels, gpui_render_cancel,
    gpui_render_submit_async, gpui_render_try_take, hold_worker_for_tests, publish_count_for_tests,
    quiesce_worker_for_tests, slot_count_for_tests, slot_state_for_tests,
};
use gpui_nim_shim::tree::{Node, NodeId};
use gpui_nim_shim::{
    gpui_append_child, gpui_create_element, gpui_create_text_node, gpui_destroy_element,
    gpui_reset_tree, gpui_set_style, GpuiElement, ROOT_NODE_ID,
};
use serial_test::serial;

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

/// Block until the worker has completed one more publish than
/// `before`, or the deadline expires. Returns whether the publish was
/// observed — the caller ASSERTS that, so "the worker never answered"
/// and "the worker answered and left nothing behind" cannot be
/// confused (§4: a wait that times out silently makes every
/// assertion after it vacuous).
fn quiesce() {
    assert!(
        quiesce_worker_for_tests(30_000),
        "the render worker still owes a publish after 30s; every \
         assertion about what a publish left behind would be about \
         somebody else's token"
    );
}

fn await_publish(before: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if publish_count_for_tests() > before {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

unsafe fn seed_root() -> *mut GpuiElement {
    gpui_reset_tree();
    let root_id: NodeId = {
        let mut tree = gpui_nim_shim::lock_tree();
        let root_node = Node::new_element("root");
        tree.insert(root_node)
    };
    {
        let mut root = ROOT_NODE_ID
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        *root = root_id;
    }
    #[repr(C)]
    struct Local {
        node_id: u64,
    }
    let boxed = Box::new(Local { node_id: root_id.0 });
    Box::into_raw(boxed) as *mut GpuiElement
}

/// Drive a single render-and-take cycle, polling try_take until the
/// worker finishes. Returns the deadline-bounded outcome.
fn render_one(width: u32, height: u32) -> (i32, Vec<u8>) {
    let token = gpui_render_submit_async(width, height, 1.0);
    assert_ne!(token, 0, "submit must return a non-zero token");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut out_ptr: *mut u8 = std::ptr::null_mut();
        let mut out_len: usize = 0;
        let rc = gpui_render_try_take(token, &mut out_ptr, &mut out_len);
        if rc == 0 {
            // Ready.
            assert!(!out_ptr.is_null(), "ready out_ptr is null");
            let bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len) }.to_vec();
            unsafe { gpui_free_pixels(out_ptr, out_len) };
            return (0, bytes);
        } else if rc == 1 {
            // Pending — keep polling.
            assert!(out_ptr.is_null());
            assert_eq!(out_len, 0);
            if Instant::now() > deadline {
                return (rc, Vec::new());
            }
            std::thread::sleep(Duration::from_millis(2));
            continue;
        } else {
            // Error / unknown token.
            return (rc, Vec::new());
        }
    }
}

#[test]
#[serial]
fn async_render_produces_non_empty_buffer() {
    unsafe {
        let root = seed_root();
        let rect = gpui_create_element(c("div").as_ptr());
        gpui_set_style(rect, c("background").as_ptr(), c("#2244aa").as_ptr());
        gpui_set_style(rect, c("width").as_ptr(), c("100%").as_ptr());
        gpui_set_style(rect, c("height").as_ptr(), c("100%").as_ptr());
        gpui_append_child(root, rect);
        // GPUI's deferred-draw pipeline needs at least one text node
        // in the tree to produce a non-empty raster; a bare styled
        // div is captured as an all-black buffer.
        let label = gpui_create_text_node(c("EMC2-M1").as_ptr());
        gpui_append_child(rect, label);

        let (rc, bytes) = render_one(120, 80);

        #[cfg(target_os = "macos")]
        {
            assert_eq!(rc, 0, "async render must succeed on macOS, got {}", rc);
            assert_eq!(bytes.len(), 120 * 80 * 4, "buffer must be width*height*4");
            let mut unique = HashSet::new();
            let mut i = 0;
            while i < bytes.len() {
                let rgba = (bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]);
                unique.insert(rgba);
                if unique.len() > 4 {
                    break;
                }
                i += 4;
            }
            eprintln!("unique colours observed: {} (first pixel: {:?})", unique.len(), unique.iter().take(4).collect::<Vec<_>>());
            assert!(
                unique.len() > 1,
                "rendered buffer must contain more than one unique colour"
            );
        }
        #[cfg(not(target_os = "macos"))]
        {
            // Linux: the worker reports RendererUnavailable, negated to -2.
            assert_eq!(rc, -2, "non-macOS expects -RendererUnavailable, got {}", rc);
            assert!(bytes.is_empty());
        }

        gpui_destroy_element(label);
        gpui_destroy_element(rect);
        gpui_destroy_element(root);
    }
}

/// The acceptance criterion: two consecutive submits must each return
/// immediately. Each call must take well under the synchronous
/// ~41 ms render-time, proving the bridge thread is not blocked.
#[test]
#[serial]
fn async_submit_is_non_blocking() {
    unsafe {
        let root = seed_root();
        let rect = gpui_create_element(c("div").as_ptr());
        gpui_set_style(rect, c("background").as_ptr(), c("#882244").as_ptr());
        gpui_set_style(rect, c("width").as_ptr(), c("100%").as_ptr());
        gpui_set_style(rect, c("height").as_ptr(), c("100%").as_ptr());
        gpui_append_child(root, rect);
        let label = gpui_create_text_node(c("EMC2-M1 non-blocking").as_ptr());
        gpui_append_child(rect, label);

        // Prime the worker with one render so the HeadlessAppContext
        // is already constructed (otherwise the first submit pays the
        // ~30 ms init cost on the worker thread; the BRIDGE thread
        // still returns immediately, but we want a clean measurement
        // of just the submit + take overhead).
        let (rc, _bytes) = render_one(120, 80);
        #[cfg(not(target_os = "macos"))]
        {
            assert_eq!(rc, -2);
            return;
        }
        #[cfg(target_os = "macos")]
        {
            assert_eq!(rc, 0);

            // Now measure two back-to-back submits. Each must return
            // in microseconds, not milliseconds.
            let t0 = Instant::now();
            let token_a = gpui_render_submit_async(120, 80, 1.0);
            let elapsed_a = t0.elapsed();
            let t1 = Instant::now();
            let token_b = gpui_render_submit_async(120, 80, 1.0);
            let elapsed_b = t1.elapsed();

            assert_ne!(token_a, 0);
            assert_ne!(token_b, 0);
            assert_ne!(token_a, token_b);
            // 5 ms is a very generous ceiling — submits should be
            // <100 us in practice. We allow 5 ms to absorb CI jitter
            // and the cost of the slot HashMap insert + Mutex lock.
            assert!(
                elapsed_a < Duration::from_millis(5),
                "first submit took {:?}, must be sub-5ms",
                elapsed_a
            );
            assert!(
                elapsed_b < Duration::from_millis(5),
                "second submit took {:?}, must be sub-5ms",
                elapsed_b
            );

            // Cancel both so the slot map doesn't accumulate.
            gpui_render_cancel(token_a);
            gpui_render_cancel(token_b);
            // Drain any frames that might have been queued meanwhile.
            std::thread::sleep(Duration::from_millis(200));
            let mut p: *mut u8 = std::ptr::null_mut();
            let mut l: usize = 0;
            let _ = gpui_render_try_take(token_a, &mut p, &mut l);
            if !p.is_null() {
                gpui_free_pixels(p, l);
            }
            let _ = gpui_render_try_take(token_b, &mut p, &mut l);
            if !p.is_null() {
                gpui_free_pixels(p, l);
            }
        }

        gpui_destroy_element(label);
        gpui_destroy_element(rect);
        gpui_destroy_element(root);
    }
}

#[test]
#[serial]
fn async_try_take_unknown_token() {
    let mut p: *mut u8 = std::ptr::null_mut();
    let mut l: usize = 0;
    // 0 token is always unknown.
    let rc = gpui_render_try_take(0, &mut p, &mut l);
    assert_eq!(rc, -100);
    assert!(p.is_null());
    assert_eq!(l, 0);
    // An arbitrary high token that was never issued is also unknown.
    let rc = gpui_render_try_take(0xDEAD_BEEF, &mut p, &mut l);
    assert_eq!(rc, -100);
}

/// ERV-M3: a token submitted BEFORE ``gpui_bump_generation`` must
/// surface as ``TAKE_STALE`` (return code 2) instead of the
/// previously-rendered bytes — so a story-switch can't paint the
/// prior story's pixels.
///
/// The Pending branch is exercised here because the production
/// race is "submit, switch story, take" — the take observes a
/// Pending or Ready slot whose snapshot is older than the live
/// generation. We assert on the Pending path because it does not
/// require the headless renderer to actually complete a frame
/// (so the test passes on non-macOS hosts that bail with
/// RendererUnavailable inside the worker).
#[test]
#[serial]
fn stale_token_after_bump_returns_stale_sentinel() {
    // Submit a render, then bump the generation BEFORE polling.
    // The slot is most likely still Pending; the try_take must
    // return TAKE_STALE = 2.
    let token = gpui_render_submit_async(64, 48, 1.0);
    assert_ne!(token, 0);

    let new_gen = gpui_bump_generation();
    assert!(new_gen >= 1, "bump must return a monotonically increasing value");

    let mut out_ptr: *mut u8 = std::ptr::null_mut();
    let mut out_len: usize = 0;
    let rc = gpui_render_try_take(token, &mut out_ptr, &mut out_len);
    // The slot was submitted at the prior generation, so try_take
    // either:
    //   * observes Pending with stale gen   → TAKE_STALE (2)
    //   * observes Ready with stale gen     → TAKE_STALE (2)
    //   * observes Failed with stale gen    → TAKE_STALE (2)
    // (The non-stale Pending value is impossible because the bump
    // happened before this poll; the worker captured the prior gen
    // at submit time.)
    assert_eq!(
        rc, 2,
        "stale token must return TAKE_STALE (2); got {}",
        rc
    );
    assert!(out_ptr.is_null());
    assert_eq!(out_len, 0);

    // Token must be consumed — a second poll returns UnknownToken.
    //
    // This assertion was a ~1-in-25 flake before the tombstone
    // existed, and the flake was a real defect rather than a timing
    // artefact: the stale branch REMOVED the Pending entry, the
    // worker's publish a few microseconds later re-created it, and
    // this poll then read `TAKE_STALE` (2) off the resurrected slot.
    // Measured before the repair: 4 failures in 200 runs under
    // 8-way load, every one of them `left: 2, right: -100` here.
    //
    // With the tombstone it is deterministic under BOTH interleavings,
    // which is what makes it a fix rather than a wider window: if the
    // worker has not published, the poll reads the tombstone; if it
    // has, the publish cleared the key. `TAKE_UNKNOWN_TOKEN` either
    // way. (Asserting the tombstone is PRESENT here would be a second
    // flake wearing the first one's clothes — it is transient by
    // design. What it leaves behind is asserted, after the publish, by
    // `abandoned_token_leaves_no_orphan_slot` below.)
    let rc = gpui_render_try_take(token, &mut out_ptr, &mut out_len);
    assert_eq!(rc, -100, "stale-consumed token must be unknown on re-poll");
}

/// PLAT-19: the other half of the tombstone, and the expensive half.
///
/// The worker's publish for an abandoned token must be DROPPED, not
/// stored. Without that, a story-switch landing on an in-flight render
/// leaves a `Ready` orphan holding `width * height * 4` bytes under a
/// token that is never polled again — for the process lifetime, once
/// per occurrence.
///
/// This is `stale_token_after_bump_returns_stale_sentinel`'s positive
/// twin (§4a): that case asserts what the caller is TOLD, this one
/// asserts what the map is LEFT HOLDING, and both run through the same
/// stale branch. Before the repair it fails at the tombstone assertion
/// with `"none"`; with only that assertion removed it then fails at the
/// final one with **`"failed"`** — measured on Linux, where the worker
/// answers `RendererUnavailable`. On macOS the orphan is a `"ready"`
/// slot with the raster still attached, which is the expensive form;
/// that has been read off the code path and NOT observed on a host, so
/// it is stated as an expectation rather than as a measurement.
///
/// **The in-flight state is CONSTRUCTED, not raced for.** `TAKE_STALE`
/// is returned whether the worker has published or not, and only the
/// not-yet case can orphan anything, so an attempt that loses the race
/// asserts something true in both worlds — green, and about nothing.
/// Two earlier shapes of this case were measured and rejected: a single
/// unguarded attempt (vacuous when it loses) and a bounded retry loop
/// (lost all 40 attempts twice in 400 runs under 8-way load, so the
/// case was a flake in its own right). Holding the worker gate removes
/// the race instead of widening the window.
#[test]
#[serial]
fn abandoned_token_leaves_no_orphan_slot() {
    quiesce();
    let before = slot_count_for_tests();
    let publishes_before = publish_count_for_tests();
    let abandons_before = abandon_count_for_tests();

    // The worker cannot publish while this is alive.
    let hold = hold_worker_for_tests();

    let token = gpui_render_submit_async(64, 48, 1.0);
    assert_ne!(token, 0);
    gpui_bump_generation();

    let mut out_ptr: *mut u8 = std::ptr::null_mut();
    let mut out_len: usize = 0;
    let rc = gpui_render_try_take(token, &mut out_ptr, &mut out_len);
    assert_eq!(rc, 2, "a token older than the live generation is stale");
    assert!(out_ptr.is_null());

    // The branch this case is about was reached — positive control, so
    // the assertions below cannot be about the already-published path.
    assert_eq!(
        abandon_count_for_tests(),
        abandons_before + 1,
        "the stale take must have gone through the in-flight branch"
    );
    assert_eq!(
        slot_state_for_tests(token),
        "abandoned",
        "a stale take of a Pending slot must TOMBSTONE it, not remove \
         it — removing it is what lets the worker's publish re-create \
         an orphan"
    );
    assert_eq!(
        gpui_render_try_take(token, &mut out_ptr, &mut out_len),
        -100,
        "the tombstone answers UnknownToken, not a second stale"
    );

    // Let the worker publish, then require that the publish was dropped.
    drop(hold);
    assert!(
        await_publish(publishes_before),
        "worker did not publish within the deadline; this case has \
         measured nothing"
    );

    assert_eq!(
        slot_state_for_tests(token),
        "none",
        "the worker's publish for an abandoned token must be dropped, \
         not stored under a token nobody will poll again"
    );
    assert_eq!(
        slot_count_for_tests(),
        before,
        "the slot map must return to its prior size"
    );
}

/// PLAT-19: `gpui_render_cancel` over an in-flight token has the same
/// shape and had the same orphan. Its doc comment claimed "no leak on
/// any branch" while the `Pending` branch was the leak. Same
/// construction, for the same reason.
#[test]
#[serial]
fn cancel_while_pending_leaves_no_orphan_slot() {
    quiesce();
    let before = slot_count_for_tests();
    let publishes_before = publish_count_for_tests();
    let abandons_before = abandon_count_for_tests();

    let hold = hold_worker_for_tests();

    let token = gpui_render_submit_async(64, 48, 1.0);
    assert_ne!(token, 0);
    gpui_render_cancel(token);

    assert_eq!(
        abandon_count_for_tests(),
        abandons_before + 1,
        "the cancel must have gone through the in-flight branch"
    );
    assert_eq!(
        slot_state_for_tests(token),
        "abandoned",
        "cancelling an in-flight token must tombstone it"
    );

    let mut out_ptr: *mut u8 = std::ptr::null_mut();
    let mut out_len: usize = 0;
    assert_eq!(
        gpui_render_try_take(token, &mut out_ptr, &mut out_len),
        -100,
        "a cancelled token is unknown to the caller from the cancel on"
    );

    drop(hold);
    assert!(
        await_publish(publishes_before),
        "worker did not publish within the deadline; this case has \
         measured nothing"
    );

    assert_eq!(
        slot_state_for_tests(token),
        "none",
        "cancel must not orphan the worker's pending publish"
    );
    assert_eq!(slot_count_for_tests(), before);
}

/// ERV-M3: ``gpui_bump_generation`` is monotonic and the value it
/// returns is observable from a subsequent submit. This is the
/// happens-before edge the staleness check depends on.
#[test]
#[serial]
fn bump_generation_is_monotonic() {
    let g0 = gpui_bump_generation();
    let g1 = gpui_bump_generation();
    let g2 = gpui_bump_generation();
    assert!(g1 > g0, "bump must increase: {} -> {}", g0, g1);
    assert!(g2 > g1, "bump must increase: {} -> {}", g1, g2);
    assert_eq!(g1, g0 + 1);
    assert_eq!(g2, g1 + 1);
}
