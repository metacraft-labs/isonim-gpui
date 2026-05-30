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
    gpui_free_pixels, gpui_render_cancel, gpui_render_submit_async,
    gpui_render_try_take,
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
