//! RS-M14 Phase 2 — Rust-side integration test for headless RGBA rendering
//! via Zed's `HeadlessAppContext`.
//!
//! Drives `gpui_render_to_pixels` against a small composition built through
//! the shim's real `extern "C"` shadow-tree API (no mocks), and asserts:
//!
//! - return code is 0 (success) on macOS;
//! - buffer length matches `width * height * 4`;
//! - the buffer contains varied content (more than one unique colour),
//!   which proves we are rendering real GPUI pixels rather than a uniform
//!   white / black canvas;
//! - `gpui_free_pixels` releases the buffer without panicking;
//! - error paths return non-zero error codes and (null, 0) out-params.
//!
//! The test only runs when the shim is built with the `gpui-headless`
//! Cargo feature.
//!
//! On Linux (where `current_headless_renderer()` returns `None` in the
//! pinned revision) `gpui_render_to_pixels` returns the
//! `RendererUnavailable` error code (2). The Linux path falls back to
//! RS-M14b (Xvfb / weston-headless) per the RS-M14 milestone scope.

#![cfg(feature = "gpui-headless")]

use std::collections::HashSet;
use std::ffi::CString;

use gpui_nim_shim::gpui_headless::{gpui_free_pixels, gpui_render_to_pixels};
use gpui_nim_shim::tree::{Node, NodeId};
use gpui_nim_shim::{
    gpui_append_child, gpui_create_element, gpui_create_text_node, gpui_destroy_element,
    gpui_reset_tree, gpui_set_style, GpuiElement, ROOT_NODE_ID,
};
use serial_test::serial;

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

/// Reset the shadow tree and seed a root node, returning a heap handle
/// for it. Mirrors the shim's `gpui_launch` setup so the
/// `NimRootView` finds a root to render.
unsafe fn seed_root() -> *mut GpuiElement {
    gpui_reset_tree();
    let root_id: NodeId = {
        let mut tree = gpui_nim_shim::lock_tree();
        let root_node = Node::new_element("root");
        tree.insert(root_node)
    };
    {
        let mut root = ROOT_NODE_ID.lock().unwrap_or_else(|p| p.into_inner());
        *root = root_id;
    }
    // Hand the caller a heap-allocated handle pointing at the root.
    // (The public GpuiElement layout is `#[repr(C)] { node_id: u64 }`.)
    #[repr(C)]
    struct Local {
        node_id: u64,
    }
    let boxed = Box::new(Local { node_id: root_id.0 });
    Box::into_raw(boxed) as *mut GpuiElement
}

/// On macOS, the headless render path runs end-to-end and the captured
/// raster must contain more than one unique colour (a uniform canvas
/// would mean the GPUI render pipeline never actually painted the shadow
/// tree).
///
/// On non-macOS platforms `current_headless_renderer()` returns `None`,
/// so we expect `RendererUnavailable` (error code 2). We never get to
/// the histogram check on those hosts.
#[test]
#[serial]
fn headless_render_produces_non_empty_buffer() {
    unsafe {
        let root = seed_root();
        // Build: root > div(background=red, width=100%, height=100%) > "Hello"
        let div_tag = c("div");
        let label_text = c("Hello RS-M14 Phase 2");

        let rect = gpui_create_element(div_tag.as_ptr());
        gpui_set_style(rect, c("background").as_ptr(), c("#dc285a").as_ptr());
        gpui_set_style(rect, c("width").as_ptr(), c("100%").as_ptr());
        gpui_set_style(rect, c("height").as_ptr(), c("100%").as_ptr());
        gpui_append_child(root, rect);

        let label = gpui_create_text_node(label_text.as_ptr());
        gpui_append_child(rect, label);

        let mut out_ptr: *mut u8 = std::ptr::null_mut();
        let mut out_len: usize = 0;

        let width: u32 = 100;
        let height: u32 = 100;
        let rc = gpui_render_to_pixels(width, height, 1.0, &mut out_ptr, &mut out_len);

        #[cfg(target_os = "macos")]
        {
            assert_eq!(rc, 0, "gpui_render_to_pixels returned non-zero error code");
            assert!(!out_ptr.is_null(), "out_ptr is null on success");
            assert_eq!(
                out_len,
                (width as usize) * (height as usize) * 4,
                "buffer length must be width * height * 4"
            );

            // Inspect the pixel buffer: the renderer should produce more
            // than one unique RGBA value. A uniform canvas would mean the
            // render pipeline failed to paint the shadow tree.
            let slice = std::slice::from_raw_parts(out_ptr, out_len);
            let mut unique = HashSet::new();
            let mut i = 0;
            while i < slice.len() {
                let rgba = (slice[i], slice[i + 1], slice[i + 2], slice[i + 3]);
                unique.insert(rgba);
                if unique.len() > 8 {
                    break;
                }
                i += 4;
            }
            assert!(
                unique.len() > 1,
                "expected rendered buffer to contain more than one unique RGBA value, got {}",
                unique.len()
            );

            gpui_free_pixels(out_ptr, out_len);
        }

        #[cfg(not(target_os = "macos"))]
        {
            // Linux / other platforms: `current_headless_renderer()`
            // returns `None` at the pinned commit, so the entry point
            // bails out with error code 2 (RendererUnavailable).
            assert_eq!(
                rc, 2,
                "non-macOS hosts expect RendererUnavailable (code 2)"
            );
            assert!(out_ptr.is_null());
            assert_eq!(out_len, 0);
        }

        // Cleanup handles we created. Tree state resets on the next test
        // iteration via `gpui_reset_tree`.
        gpui_destroy_element(rect);
        gpui_destroy_element(label);
        gpui_destroy_element(root);
    }
}

#[test]
#[serial]
fn headless_render_rejects_zero_dimensions() {
    unsafe {
        let _root = seed_root();
        let mut out_ptr: *mut u8 = std::ptr::null_mut();
        let mut out_len: usize = 0;
        let rc = gpui_render_to_pixels(0, 100, 1.0, &mut out_ptr, &mut out_len);
        assert_ne!(rc, 0, "zero width should produce an error");
        assert!(out_ptr.is_null());
        assert_eq!(out_len, 0);
    }
}

#[test]
#[serial]
fn headless_render_rejects_invalid_scale() {
    unsafe {
        let _root = seed_root();
        let mut out_ptr: *mut u8 = std::ptr::null_mut();
        let mut out_len: usize = 0;
        let rc = gpui_render_to_pixels(10, 10, -1.0, &mut out_ptr, &mut out_len);
        assert_ne!(rc, 0, "negative scale should produce an error");
        assert!(out_ptr.is_null());
        assert_eq!(out_len, 0);

        let rc = gpui_render_to_pixels(10, 10, 0.0, &mut out_ptr, &mut out_len);
        assert_ne!(rc, 0, "zero scale should produce an error");
    }
}

#[test]
#[serial]
fn headless_free_pixels_is_null_safe() {
    unsafe {
        // Should not panic / segfault.
        gpui_free_pixels(std::ptr::null_mut(), 0);
        gpui_free_pixels(std::ptr::null_mut(), 1024);
    }
}
