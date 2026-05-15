//! RS-M14 Phase 2: headless RGBA rendering via Zed's `HeadlessAppContext`.
//!
//! Used by isonim-render-serve's GPUI adapter to deliver real GPUI pixels
//! through the F/M/I bridge instead of the synthetic vertical-stack stripes
//! the pre-RS-M14 adapter produced. Runs the same GPUI render pipeline the
//! windowed path uses, just captured via `Window::render_to_image()`
//! (test-support-gated) into an off-screen `image::RgbaImage` rather than
//! presenting to an OS swapchain.
//!
//! ## Production-path preservation
//!
//! This module is behind the `gpui-headless` Cargo feature and does not
//! touch the existing windowed launch path in `gpui_app.rs`. Builds that
//! only enable `gpui-backend` (the windowed path) do not pay for the
//! `HeadlessAppContext` / `MetalHeadlessRenderer` machinery; builds that
//! enable both can use either entry point.
//!
//! ## Pin
//!
//! The headless API (`HeadlessAppContext::with_platform`,
//! `Window::render_to_image`, `MetalHeadlessRenderer`) is unpublished on
//! crates.io. The shim git-pins zed-industries/zed at-or-after PR #55059
//! (2026-04-28, "Query for window instead of capturing"). The exact
//! revision lives in `Cargo.toml`.
//!
//! ## Color space + byte order
//!
//! `Window::render_to_image()` returns an `image::RgbaImage`, which is
//! RGBA8888 non-premultiplied sRGB by construction. No manual BGRA↔RGBA
//! conversion is needed; the bytes can be fed directly into a browser
//! `canvas.putImageData(ImageData(...))` call.

use std::sync::Arc;

use gpui::{px, size, AnyWindowHandle, App, AppContext, HeadlessAppContext, Pixels, Size, Window};
use gpui_platform::{current_headless_renderer, current_platform};

use crate::gpui_app::NimRootView;

/// Upper bound on a single output dimension. Defended against
/// (width * height * 4) overflow; 16384 * 16384 * 4 = 1 GiB which is
/// already absurdly large for an off-screen UI snapshot.
const MAX_DIMENSION: u32 = 16_384;

/// Error codes returned across the FFI boundary. Values are stable and
/// documented in the spec; callers can compare against the numeric
/// constants directly.
#[repr(i32)]
enum ErrorCode {
    InvalidArgs = 1,
    /// `current_headless_renderer()` returned `None` for the current
    /// platform. macOS is the supported headless target; Linux falls back
    /// to RS-M14b (Xvfb / weston-headless) per the milestone scope.
    RendererUnavailable = 2,
    /// Opening a window via `HeadlessAppContext::open_window` failed.
    WindowOpenFailed = 3,
    /// `capture_screenshot` returned an error (typically the renderer
    /// could not produce a frame — e.g. `Window::render_to_image` was
    /// called before the first draw cycle).
    CaptureFailed = 4,
    /// The captured image dimensions did not match the caller's request.
    /// Surfaced as an error so callers can fall back to the synthetic
    /// stripes path rather than silently returning a buffer whose shape
    /// disagrees with what was advertised.
    SizeMismatch = 5,
    /// A Rust panic propagated up the FFI boundary. The buffer pointer
    /// + length are set to (null, 0) before the function returns.
    Panic = 6,
}

/// Render the current shadow tree to RGBA8888 pixels at the specified
/// size.
///
/// Returns 0 on success and writes the pixel-buffer pointer to
/// `*out_ptr` + byte count to `*out_len`. The buffer is allocated by
/// this function and MUST be released via [`gpui_free_pixels`]; the
/// caller owns the buffer for the duration between the two calls.
///
/// Returns a non-zero error code on failure; on error the function
/// writes a null pointer + zero length to the caller's out parameters
/// so naive callers that always free the buffer remain safe.
///
/// # Color space + byte order
///
/// The output is RGBA8888 non-premultiplied sRGB, row-major, top row
/// first — the canonical F-packet pixel format. `image::RgbaImage` is
/// already in this layout so no conversion is performed.
///
/// # Scale semantics
///
/// `width` and `height` are the output pixel dimensions. `scale` is the
/// logical-to-physical ratio: the window is opened at
/// `(width / scale, height / scale)` logical pixels. GPUI's pinned
/// rendering pipeline currently captures at the logical size; callers
/// in this milestone pass `scale = 1.0` so the logical and physical
/// dimensions line up.
///
/// # Safety
///
/// `out_ptr` and `out_len` MUST be non-null and point to writable
/// storage. The caller MUST NOT read or free the output buffer if the
/// function returns a non-zero error code.
#[no_mangle]
pub extern "C" fn gpui_render_to_pixels(
    width: u32,
    height: u32,
    scale: f32,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    // 1. Argument validation. Bail before allocating any GPUI resources.
    if out_ptr.is_null() || out_len.is_null() {
        return ErrorCode::InvalidArgs as i32;
    }
    // Initialise the out parameters to (null, 0) so the caller can safely
    // call `gpui_free_pixels` even after an error.
    unsafe {
        *out_ptr = std::ptr::null_mut();
        *out_len = 0;
    }
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return ErrorCode::InvalidArgs as i32;
    }
    if !scale.is_finite() || scale <= 0.0 {
        return ErrorCode::InvalidArgs as i32;
    }

    // 2. Catch panics across the FFI boundary — unwinding into Nim is UB.
    //    The HeadlessAppContext / Metal renderer interacts with a lot of
    //    platform code; any pathological state (missing fonts, GPU device
    //    init failure on a CI runner) surfaces as a panic in practice.
    let result = std::panic::catch_unwind(|| render_to_rgba(width, height, scale));

    let rgba = match result {
        Ok(Ok(rgba)) => rgba,
        Ok(Err(code)) => return code as i32,
        Err(_) => return ErrorCode::Panic as i32,
    };

    // 3. Hand the buffer over to the caller. `Box::leak` keeps the
    //    allocation alive until `gpui_free_pixels` reclaims it.
    let len = rgba.len();
    let boxed: Box<[u8]> = rgba.into_boxed_slice();
    let ptr = Box::leak(boxed).as_mut_ptr();
    unsafe {
        *out_ptr = ptr;
        *out_len = len;
    }
    0
}

/// Free a buffer previously returned by [`gpui_render_to_pixels`].
///
/// # Safety
///
/// `ptr` MUST be a pointer returned by `gpui_render_to_pixels`, and
/// `len` MUST be the byte count associated with that allocation.
/// Calling with `ptr == null` or `len == 0` is a no-op. Calling twice
/// on the same buffer is undefined behaviour (double-free).
#[no_mangle]
pub unsafe extern "C" fn gpui_free_pixels(ptr: *mut u8, len: usize) {
    if ptr.is_null() || len == 0 {
        return;
    }
    let slice = std::slice::from_raw_parts_mut(ptr, len);
    drop(Box::from_raw(slice));
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Hard-coded scale factor of the `TestPlatform` test window
/// (`TestWindow::scale_factor()` in the pinned Zed revision returns 2.0).
/// `render_to_image` multiplies the logical window size by this factor when
/// reading back the framebuffer, so we have to divide the caller-requested
/// physical dimensions by it to open a window whose capture matches the
/// requested output size.
const TEST_WINDOW_SCALE_FACTOR: f32 = 2.0;

/// Drive `HeadlessAppContext` to produce raw RGBA bytes from the current
/// shadow tree.
fn render_to_rgba(width: u32, height: u32, scale: f32) -> Result<Vec<u8>, ErrorCode> {
    // The caller specifies (width, height) in *output / physical* pixels.
    // GPUI's `TestWindow::render_to_image` returns an image whose dimensions
    // are `logical_size * scale_factor`, where the test platform's
    // `scale_factor` is hard-coded to 2.0. So we open the window at
    // `(width, height) / TEST_WINDOW_SCALE_FACTOR` logical pixels and
    // expect a `(width, height)` physical capture.
    //
    // The caller-supplied `scale` argument is currently informational
    // (validated against zero / non-finite); the F/M/I bridge always passes
    // 1.0. A future revision can introduce explicit DPR control once GPUI
    // exposes a configurable headless scale factor.
    let _ = scale; // not yet plumbed through the test platform.
    let logical_w = (width as f32 / TEST_WINDOW_SCALE_FACTOR).max(1.0);
    let logical_h = (height as f32 / TEST_WINDOW_SCALE_FACTOR).max(1.0);
    let window_size: Size<Pixels> = size(px(logical_w), px(logical_h));

    // The platform's text system is what `MacTextSystem` (on macOS) /
    // `CosmicTextSystem` (on Linux, when wired up) provide; we obtain it
    // from `current_platform(true)` (headless mode) rather than
    // instantiating directly so the same code path works across OSes.
    //
    // `current_platform(true)` returns an `Rc<dyn Platform>` whose
    // `.text_system()` accessor yields the shared `Arc<dyn
    // PlatformTextSystem>` the platform's renderer is configured against.
    let platform = current_platform(true);
    let text_system = platform.text_system();

    // The headless renderer factory is platform-specific. On macOS it
    // returns `Some(MetalHeadlessRenderer)`; on Linux it returns `None`
    // in the pinned revision (the Vulkan/wgpu headless path was
    // unimplemented at PR #55059). We surface the missing renderer as a
    // dedicated error code so the Nim adapter can fall back to the
    // pre-RS-M14 synthetic raster gracefully.
    if current_headless_renderer().is_none() {
        return Err(ErrorCode::RendererUnavailable);
    }

    let mut cx = HeadlessAppContext::with_platform(
        text_system,
        Arc::new(()),
        || current_headless_renderer(),
    );

    let window: AnyWindowHandle = cx
        .open_window(window_size, |_, cx: &mut App| {
            cx.new(|_| NimRootView::new())
        })
        .map_err(|_| ErrorCode::WindowOpenFailed)?
        .into();

    // Drive pending tasks (notably the deferred draw of the first frame).
    // Without this, `render_to_image` runs against an empty scene and
    // produces a blank (all-black) capture. We pump the dispatcher
    // multiple times because the layout-then-paint pipeline interleaves
    // `cx.spawn`'d tasks with the window's render schedule, and a
    // single `run_until_parked` only drains the first wave. Two
    // refresh / drain pairs is enough for complex compositions like
    // `task_app` to settle.
    cx.run_until_parked();
    refresh_window(&mut cx, window);
    cx.run_until_parked();
    refresh_window(&mut cx, window);
    cx.run_until_parked();

    // Capture: `capture_screenshot` is the public surface that wraps
    // `app.update_window(window, |_, window, _| window.render_to_image())`.
    // The returned `RgbaImage` is RGBA8888 non-premultiplied sRGB.
    let image = cx
        .capture_screenshot(window)
        .map_err(|_| ErrorCode::CaptureFailed)?;

    let actual_w = image.width();
    let actual_h = image.height();
    if actual_w != width || actual_h != height {
        return Err(ErrorCode::SizeMismatch);
    }

    Ok(image.into_raw())
}

/// Refresh the window after first opening so the deferred draw produces
/// content the next `run_until_parked` can capture.
///
/// `Window::refresh` schedules a re-render on the next tick; combined
/// with `run_until_parked` it forces the headless renderer to actually
/// draw the scene before `capture_screenshot` reads back the texture.
fn refresh_window(cx: &mut HeadlessAppContext, window: AnyWindowHandle) {
    let _ = cx.update_window(window, |_, window: &mut Window, _| {
        window.refresh();
    });
}
