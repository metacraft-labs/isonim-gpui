//! RS-M14 Phase 2: headless RGBA rendering via Zed's `HeadlessAppContext`.
//!
//! Used by isonim-render-serve's GPUI adapter to deliver real GPUI pixels
//! through the F/M/I bridge instead of the synthetic vertical-stack stripes
//! the pre-RS-M14 adapter produced. Runs the same GPUI render pipeline the
//! windowed path uses, just captured via `Window::render_to_image()`
//! (test-support-gated) into an off-screen `image::RgbaImage` rather than
//! presenting to an OS swapchain.
//!
//! ## ERV-M3: story-generation guard against stale in-flight renders
//!
//! The async worker thread holds a long-lived `HeadlessAppContext` and
//! reuses an `AnyWindowHandle` across frames. When the bridge mutates
//! the shadow tree because the user picked a new story via
//! `select-story`, the worker may already have an in-flight render
//! submitted against the prior tree state. That render completes and
//! its bytes would be returned to the bridge — painting the wrong
//! story for a tick or two before the next render lands.
//!
//! The guard is a process-wide `AtomicU64` `current_generation`. Each
//! `gpui_render_submit_async` snapshots the value at submit time into
//! the per-token slot. The Nim adapter calls
//! `gpui_bump_generation()` BEFORE mutating the GPUI tree (i.e. inside
//! its `select-story` handler). Subsequent `gpui_render_try_take`
//! calls compare the slot's snapshot against the live counter; if the
//! snapshot is older the bytes are freed and a `TAKE_STALE` sentinel
//! is returned. The bridge treats `TAKE_STALE` like Pending (no frame
//! to paint this tick) and submits a fresh request whose snapshot is
//! current.
//!
//! Single shared generation suffices because the worker is a single
//! global; there is no per-connection or per-handle generation
//! counter (the FFI surface has no handle parameter). Future revisions
//! that introduce per-connection workers can shard the counter then.
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

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU32, AtomicU64, Ordering},
    mpsc, Arc, LazyLock, Mutex,
};
use std::thread;

use gpui::{px, size, AnyWindowHandle, App, AppContext, HeadlessAppContext, Pixels, Size, Window};
use gpui_platform::{current_headless_renderer, current_platform};
use image::imageops::FilterType;

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
/// `width` and `height` are both the **design canvas** in logical (dp)
/// units AND the output pixel dimensions — the F/M/I bridge passes the
/// browser canvas's CSS-pixel size and treats received frame bytes as
/// the same canvas at 1:1. The GPUI window is opened at `(width,
/// height)` logical pixels so leaves laid out in dp units (`padding:
/// 10`, `gap: 8`, etc.) occupy the proportion of the design canvas the
/// author intended.
///
/// The pinned Zed test platform hard-codes its scale factor to 2.0, so
/// the headless capture produces a `(2*width, 2*height)` device-pixel
/// image internally; the shim downsamples that with a triangle filter
/// to the requested `(width, height)` before returning so the caller
/// always receives `width * height * 4` bytes.
///
/// The `scale` argument is currently informational (validated against
/// zero / non-finite); the F/M/I bridge always passes 1.0. A future
/// revision can introduce explicit DPR control once GPUI exposes a
/// configurable headless scale factor.
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
/// reading back the framebuffer, so the captured image is always twice as
/// wide and twice as tall as the logical window we open.
const TEST_WINDOW_SCALE_FACTOR: f32 = 2.0;

/// Drive `HeadlessAppContext` to produce raw RGBA bytes from the current
/// shadow tree.
fn render_to_rgba(width: u32, height: u32, scale: f32) -> Result<Vec<u8>, ErrorCode> {
    // The caller specifies (width, height) as the **design canvas** in
    // logical (dp) units — same convention as CSS px / GPUI `Pixels`.
    // The F-packet protocol's pixel dimensions match the design canvas
    // 1:1 (the F/M/I bridge always passes scale = 1.0; the browser
    // canvas treats received pixels as a CSS-pixel buffer).
    //
    // Previous implementation incorrectly divided width/height by the
    // TestWindow scale factor before opening the window. That collapsed
    // the design canvas to half its size in both dimensions, so a button
    // styled with `padding: 10` (dp) occupied four times the canvas area
    // it was meant to — the visible "everything is 4× too large" bug
    // reported by M-EVP-14.
    //
    // Correct behaviour: open the window at the full requested
    // (width, height) logical pixels so GPUI lays out elements against
    // the caller-specified design canvas. The headless renderer then
    // produces an oversampled `(2W, 2H)` device-pixel image (the test
    // platform's hard-coded 2.0 scale factor), which we downsample to
    // `(W, H)` with a triangle filter before returning to the caller.
    //
    // The caller-supplied `scale` argument remains informational
    // (validated against zero / non-finite); explicit DPR control would
    // require the test platform to expose a configurable scale factor,
    // which the pinned Zed revision does not.
    let _ = scale; // not yet plumbed through the test platform.
    let logical_w = (width as f32).max(1.0);
    let logical_h = (height as f32).max(1.0);
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
    // The returned `RgbaImage` is RGBA8888 non-premultiplied sRGB. With
    // a logical window of (W, H) and the test platform's hard-coded
    // scale_factor of 2.0, the captured image is (2W, 2H).
    let image = cx
        .capture_screenshot(window)
        .map_err(|_| ErrorCode::CaptureFailed)?;

    let expected_scale = TEST_WINDOW_SCALE_FACTOR as u32;
    let expected_capture_w = width.saturating_mul(expected_scale);
    let expected_capture_h = height.saturating_mul(expected_scale);
    let actual_w = image.width();
    let actual_h = image.height();

    // If the captured image dimensions match the requested output
    // exactly (e.g. a future GPUI revision changes the test platform
    // scale_factor to 1.0), skip the downsample step.
    if actual_w == width && actual_h == height {
        return Ok(image.into_raw());
    }

    // Otherwise verify the capture matches `request × scale_factor`,
    // then downsample by `scale_factor` to produce the (W, H) buffer
    // the caller asked for. A triangle filter (bilinear-equivalent)
    // gives clean text anti-aliasing without the blur of Lanczos.
    if actual_w != expected_capture_w || actual_h != expected_capture_h {
        return Err(ErrorCode::SizeMismatch);
    }

    let downsampled = image::imageops::resize(
        &image,
        width,
        height,
        FilterType::Triangle,
    );

    Ok(downsampled.into_raw())
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

// ===========================================================================
// EMC2-M1: dedicated GPUI render thread (Approach 1).
// ===========================================================================
//
// Background
// ----------
//
// EMC-M1 audit measured ``gpui_render_to_pixels`` at 41-43 ms median per
// frame on the bridge's async ``frameLoop`` thread. The bridge's
// per-connection frame loop is single-threaded and yields only at FFI
// boundaries that internally call ``await sendBinary`` etc., so the
// entire 41 ms shim body blocks the loop and prevents the matrix's
// 50 ms inter-frame gate from being met.
//
// ``HeadlessAppContext`` from Zed is built on top of ``Rc<AppCell>``
// (``RefCell``-backed, ``!Send``, ``!Sync``). That forbids moving an
// existing context across threads — but it does NOT forbid creating
// the context on a dedicated worker thread and keeping all subsequent
// interaction on that same thread. The ``TREE`` shadow tree is a
// ``Mutex<Tree>`` (Send + Sync), so the worker thread can read it
// freely; the FFI surface is the only thing that needs marshalling.
//
// Design
// ------
//
// The first call to ``gpui_render_submit_async`` lazily spawns a
// **single, long-lived worker thread**. The worker:
//
//   1. Constructs ``HeadlessAppContext::with_platform`` **once**
//      (EMC-M1's biggest single cost item — paid once instead of per
//      frame).
//   2. Opens a window at the first-requested size and keeps it open
//      across frames; resizes (reopens) on demand when the requested
//      ``(width, height)`` changes. Window reuse skips the pump-from-
//      scratch cost; consecutive same-size requests just refresh and
//      re-capture.
//   3. Loops on a ``mpsc::Receiver<RenderRequest>``: when a request
//      arrives, runs the existing pump-refresh-pump-refresh-pump
//      sequence then ``capture_screenshot`` + downsample. The result
//      (Vec<u8>) is stored in a per-token slot.
//   4. Wakes any waiter blocked on the per-token ``Condvar`` (used
//      only by the optional ``gpui_render_wait_async`` API; the
//      production path is non-blocking ``gpui_render_try_take``).
//
// FFI surface
// -----------
//
//   * ``gpui_render_submit_async(w, h, scale) -> u32 token``
//     Submits a render request and returns immediately. The token
//     identifies the in-flight render for ``try_take``/``cancel``.
//     A token value of 0 indicates submission failure (e.g. the
//     worker thread is stopped); the caller should fall back.
//
//   * ``gpui_render_try_take(token, out_ptr, out_len) -> i32``
//     Non-blocking; returns:
//       * ``0`` if the render is complete and writes (ptr, len) to
//         the out-params. The buffer is shim-owned until
//         ``gpui_free_pixels`` returns it (same convention as the
//         sync API).
//       * ``1`` if the render is still in flight; out-params are
//         set to (null, 0).
//       * The negative of the per-render error code if the worker
//         thread reported failure (e.g. ``-2`` for
//         RendererUnavailable). The token is consumed in this
//         case too.
//       * ``-100`` if the token is unknown (already taken, never
//         submitted, or expired).
//
// The Nim adapter is expected to pipeline:
//   - Tick N: submit frame N+1 (immediate return)
//   - Tick N: try_take frame N-1 (typically ready, sometimes not)
//   - If not ready: emit the previous frame again (smoother than
//     blocking the bridge loop on the 41 ms wait).

/// A token-keyed slot for an in-flight render's result. Each variant
/// carries the **generation** the request was submitted at, so a
/// ``try_take`` against a slot whose generation is older than the
/// live ``current_generation`` can detect the staleness and discard
/// the bytes (ERV-M3).
enum RenderSlot {
    Pending { generation: u64 },
    Ready { generation: u64, bytes: Vec<u8> },
    Failed { generation: u64, code: ErrorCode },
}

struct WorkerState {
    /// Per-token result slots. The worker fills entries here when a
    /// render completes; ``gpui_render_try_take`` drains them.
    slots: Mutex<HashMap<u32, RenderSlot>>,
    /// Channel sender. ``None`` after the worker thread has been
    /// asked to shut down (used by tests; the production path
    /// keeps the worker alive for the process lifetime).
    sender: Mutex<Option<mpsc::Sender<RenderRequest>>>,
    /// Monotonic token counter. Zero is reserved for "submission
    /// failed".
    next_token: AtomicU32,
    /// Monotonic story / tree generation counter. Bumped by
    /// ``gpui_bump_generation`` (called by the Nim adapter's
    /// ``select-story`` handler BEFORE the GPUI shadow tree mutates).
    /// ``gpui_render_submit_async`` snapshots this into the per-token
    /// slot; ``gpui_render_try_take`` rejects slots whose snapshot
    /// is older than the live value. ERV-M3.
    current_generation: AtomicU64,
}

struct RenderRequest {
    token: u32,
    width: u32,
    height: u32,
    scale: f32,
    /// Snapshot of ``WorkerState::current_generation`` taken at
    /// submit time. Worker echoes it back into the completed slot so
    /// ``try_take`` can compare against the live counter and detect
    /// stale frames produced for a tree state that has since been
    /// superseded by a story-switch.
    generation: u64,
}

static WORKER: LazyLock<Arc<WorkerState>> = LazyLock::new(|| {
    let state = Arc::new(WorkerState {
        slots: Mutex::new(HashMap::new()),
        sender: Mutex::new(None),
        next_token: AtomicU32::new(1),
        current_generation: AtomicU64::new(0),
    });
    spawn_worker(state.clone());
    state
});

fn spawn_worker(state: Arc<WorkerState>) {
    let (tx, rx) = mpsc::channel::<RenderRequest>();
    {
        let mut sender_slot = state.sender.lock().unwrap_or_else(|p| p.into_inner());
        *sender_slot = Some(tx);
    }
    // Dedicated GPUI render thread. The thread lives for the process
    // lifetime — there is no shutdown path in production. Tests that
    // need to inspect worker state use the slot APIs directly.
    let state_for_thread = state.clone();
    thread::Builder::new()
        .name("isonim-gpui-render".into())
        .spawn(move || worker_main(state_for_thread, rx))
        .expect("failed to spawn isonim-gpui-render worker thread");
}

/// Worker-thread main loop. Owns the long-lived ``HeadlessAppContext``
/// and the cached ``AnyWindowHandle`` across frames. Drains the
/// request channel forever; terminates only when the sender side is
/// dropped (the static ``WORKER`` never drops in production, so this
/// is effectively process-lifetime).
///
/// **Reuse strategy.** EMC-M1 audit found that ``HeadlessAppContext::
/// with_platform`` is the single most expensive line in the
/// synchronous render path (~30 ms of the 41 ms median, primarily
/// font-system + Metal renderer-factory init). Caching the context
/// across frames eliminates that cost from every subsequent render.
/// The window is also cached: same-size requests reuse it, so the
/// expensive ``open_window`` + first-frame pump pays once per
/// (W, H) pair. A resize closes the prior window before opening a
/// fresh one at the new size, preserving correctness.
///
/// **Render panic isolation.** Each request runs inside
/// ``catch_unwind`` so a panic in one render does not poison the
/// worker thread. After a panic the cached context is torn down;
/// the next request rebuilds it from scratch.
fn worker_main(state: Arc<WorkerState>, rx: mpsc::Receiver<RenderRequest>) {
    let mut cached: Option<WorkerCtx> = None;

    while let Ok(req) = rx.recv() {
        let token = req.token;
        let generation = req.generation;

        // Validate args at the front of the request (mirrors the
        // checks ``gpui_render_to_pixels`` does on the bridge
        // thread).
        if req.width == 0
            || req.height == 0
            || req.width > MAX_DIMENSION
            || req.height > MAX_DIMENSION
            || !req.scale.is_finite()
            || req.scale <= 0.0
        {
            publish_failure(&state, token, generation, ErrorCode::InvalidArgs);
            continue;
        }

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            render_via_cached(&mut cached, req.width, req.height)
        }));
        match result {
            Ok(Ok(bytes)) => publish_success(&state, token, generation, bytes),
            Ok(Err(code)) => {
                // Tear down on RendererUnavailable / CaptureFailed
                // so the next request retries from scratch — the
                // failure may be transient (e.g. a GPU eviction).
                cached = None;
                publish_failure(&state, token, generation, code);
            }
            Err(_) => {
                cached = None;
                publish_failure(&state, token, generation, ErrorCode::Panic);
            }
        }
    }
}

/// Per-worker cached render context. Holds the long-lived
/// ``HeadlessAppContext`` plus the most-recently-opened window's
/// dimensions and handle.
struct WorkerCtx {
    cx: HeadlessAppContext,
    window_size: Option<(u32, u32)>,
    window: Option<AnyWindowHandle>,
}

/// Render via the cached context if available; lazily construct on
/// first call. Reopens the window on size change.
fn render_via_cached(
    cached: &mut Option<WorkerCtx>,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, ErrorCode> {
    if cached.is_none() {
        if current_headless_renderer().is_none() {
            return Err(ErrorCode::RendererUnavailable);
        }
        let platform = current_platform(true);
        let text_system = platform.text_system();
        let cx = HeadlessAppContext::with_platform(
            text_system,
            Arc::new(()),
            || current_headless_renderer(),
        );
        *cached = Some(WorkerCtx {
            cx,
            window_size: None,
            window: None,
        });
    }
    let entry = cached.as_mut().expect("cached is Some after lazy init");

    let need_open = match entry.window_size {
        Some((w, h)) => w != width || h != height,
        None => true,
    };
    if need_open {
        // Close the previous window if any so the test platform's
        // window registry doesn't accumulate stale entries.
        if let Some(prev) = entry.window {
            let _ = entry.cx.update_window(prev, |_, window: &mut Window, _| {
                window.remove_window();
            });
            entry.cx.run_until_parked();
        }
        let logical_w = (width as f32).max(1.0);
        let logical_h = (height as f32).max(1.0);
        let window_size: Size<Pixels> = size(px(logical_w), px(logical_h));
        let new_window: AnyWindowHandle = entry
            .cx
            .open_window(window_size, |_, cx: &mut App| {
                cx.new(|_| NimRootView::new())
            })
            .map_err(|_| ErrorCode::WindowOpenFailed)?
            .into();
        // Prime the deferred-draw schedule so the first capture has
        // content. Same pump pattern as the synchronous path.
        entry.cx.run_until_parked();
        refresh_window(&mut entry.cx, new_window);
        entry.cx.run_until_parked();
        refresh_window(&mut entry.cx, new_window);
        entry.cx.run_until_parked();
        entry.window = Some(new_window);
        entry.window_size = Some((width, height));
    }
    let window = entry.window.expect("window is Some after open");

    // On reused-window calls the tree may have mutated; re-pump so
    // ``NimRootView::render`` re-reads the shadow tree and a fresh
    // scene is encoded into the next capture.
    refresh_window(&mut entry.cx, window);
    entry.cx.run_until_parked();
    refresh_window(&mut entry.cx, window);
    entry.cx.run_until_parked();

    let image = entry
        .cx
        .capture_screenshot(window)
        .map_err(|_| ErrorCode::CaptureFailed)?;

    let expected_scale = TEST_WINDOW_SCALE_FACTOR as u32;
    let expected_capture_w = width.saturating_mul(expected_scale);
    let expected_capture_h = height.saturating_mul(expected_scale);
    let actual_w = image.width();
    let actual_h = image.height();

    if actual_w == width && actual_h == height {
        return Ok(image.into_raw());
    }
    if actual_w != expected_capture_w || actual_h != expected_capture_h {
        return Err(ErrorCode::SizeMismatch);
    }
    let downsampled =
        image::imageops::resize(&image, width, height, FilterType::Triangle);
    Ok(downsampled.into_raw())
}

fn publish_success(state: &WorkerState, token: u32, generation: u64, bytes: Vec<u8>) {
    let mut slots = state.slots.lock().unwrap_or_else(|p| p.into_inner());
    slots.insert(token, RenderSlot::Ready { generation, bytes });
}

fn publish_failure(state: &WorkerState, token: u32, generation: u64, code: ErrorCode) {
    let mut slots = state.slots.lock().unwrap_or_else(|p| p.into_inner());
    slots.insert(token, RenderSlot::Failed { generation, code });
}

/// Submit a render request to the dedicated GPUI render thread.
///
/// Returns a non-zero token on success; the caller can poll for the
/// result via [`gpui_render_try_take`]. Returns ``0`` if the worker
/// thread is not running or the request channel is closed (callers
/// should fall back to the synchronous [`gpui_render_to_pixels`] in
/// that rare event).
///
/// This function does not block — it places the request on a channel
/// and returns immediately. The 41 ms render cost is paid on the
/// dedicated worker thread, freeing the bridge's async ``frameLoop``
/// to continue ticking.
#[no_mangle]
pub extern "C" fn gpui_render_submit_async(width: u32, height: u32, scale: f32) -> u32 {
    let state = WORKER.clone();
    let token = state.next_token.fetch_add(1, Ordering::AcqRel);
    // Reserve token 0 for failure; if the counter wraps, skip 0.
    let token = if token == 0 {
        state.next_token.fetch_add(1, Ordering::AcqRel)
    } else {
        token
    };
    // ERV-M3: snapshot the live story generation at submit time. The
    // worker echoes this back into the completed slot so
    // ``gpui_render_try_take`` can compare against the live counter
    // and drop bytes rendered against a tree state that has since
    // been superseded by a story-switch. ``Acquire`` pairs with the
    // ``Release`` ordering in ``gpui_bump_generation`` so a submit
    // that happens-after a bump observes the new generation.
    let generation = state.current_generation.load(Ordering::Acquire);
    // Mark the slot Pending so try_take can distinguish "in flight"
    // from "unknown token". Carry the generation through so a stale
    // poll on the Pending state (before the worker has rendered)
    // is still detectable.
    {
        let mut slots = state.slots.lock().unwrap_or_else(|p| p.into_inner());
        slots.insert(token, RenderSlot::Pending { generation });
    }
    let sender = {
        let guard = state.sender.lock().unwrap_or_else(|p| p.into_inner());
        guard.clone()
    };
    match sender {
        Some(s) => {
            if s.send(RenderRequest {
                token,
                width,
                height,
                scale,
                generation,
            })
            .is_err()
            {
                // Receiver dropped — clear the reserved slot and
                // report submission failure.
                let mut slots = state.slots.lock().unwrap_or_else(|p| p.into_inner());
                slots.remove(&token);
                return 0;
            }
            token
        }
        None => {
            let mut slots = state.slots.lock().unwrap_or_else(|p| p.into_inner());
            slots.remove(&token);
            0
        }
    }
}

/// ERV-M3: bump the shared story-generation counter and return the
/// new value.
///
/// Called by the Nim adapter's ``select-story`` handler BEFORE the
/// GPUI shadow tree mutates. Any render request whose snapshot
/// generation predates the bump is considered stale; its bytes are
/// discarded by ``gpui_render_try_take`` and the bridge submits a
/// fresh render against the new tree state instead of painting the
/// previous story.
///
/// Uses ``AcqRel`` so the bump synchronises-with the ``Acquire``
/// load in ``gpui_render_submit_async``: a subsequent submit on
/// any thread is guaranteed to observe the new generation.
///
/// The single global counter matches the single global worker; if
/// per-connection workers are ever introduced this will need to
/// shard. The current FFI surface has no handle parameter, so the
/// bump is process-wide.
#[no_mangle]
pub extern "C" fn gpui_bump_generation() -> u64 {
    let state = WORKER.clone();
    // ``fetch_add`` returns the prior value; add one to obtain the
    // post-bump generation that submits will observe.
    state.current_generation.fetch_add(1, Ordering::AcqRel) + 1
}

/// Per-call result code for [`gpui_render_try_take`]. ``Ready = 0``
/// matches the sync API's success convention; ``Pending = 1`` is a
/// non-error retry signal; ``Stale = 2`` (ERV-M3) means the bytes
/// were rendered against a tree state superseded by a later
/// ``gpui_bump_generation`` call (e.g. a story-switch) and were
/// discarded; negative values are the negation of the per-render
/// [`ErrorCode`] that the worker reported, plus a sentinel
/// ``-100`` for unknown tokens.
const TAKE_READY: i32 = 0;
const TAKE_PENDING: i32 = 1;
const TAKE_STALE: i32 = 2;
const TAKE_UNKNOWN_TOKEN: i32 = -100;

/// Non-blocking poll for an in-flight render's bytes.
///
/// Returns:
///   * ``0``  — render complete; ``*out_ptr`` + ``*out_len`` carry
///     the RGBA buffer (shim-owned; release via [`gpui_free_pixels`]).
///     The token is consumed.
///   * ``1``  — render still in flight; ``*out_ptr`` / ``*out_len``
///     are set to (null, 0). The token remains valid; the caller
///     should poll again on the next tick.
///   * ``2``  — ERV-M3: the slot was either Pending or Ready but
///     its captured generation predates the current generation
///     (a story-switch happened after submit). The bytes (if any)
///     were freed; the token is consumed. ``*out_ptr`` /
///     ``*out_len`` are set to (null, 0). The caller should treat
///     this exactly like Pending — submit a fresh render against
///     the new tree state instead of painting the stale frame.
///   * ``-N``  — render failed with [`ErrorCode`] N; the token is
///     consumed.
///   * ``-100`` — unknown token (never submitted, already taken,
///     or expired); the token is consumed.
///
/// # Safety
///
/// ``out_ptr`` and ``out_len`` MUST be non-null and point to writable
/// storage. On a non-zero (or pending) return code, the caller MUST
/// NOT read or free the output buffer.
#[no_mangle]
pub extern "C" fn gpui_render_try_take(
    token: u32,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    if out_ptr.is_null() || out_len.is_null() {
        return -(ErrorCode::InvalidArgs as i32);
    }
    unsafe {
        *out_ptr = std::ptr::null_mut();
        *out_len = 0;
    }
    if token == 0 {
        return TAKE_UNKNOWN_TOKEN;
    }
    let state = WORKER.clone();
    // Snapshot the live generation OUTSIDE the slots lock so the
    // comparison below is free of cross-mutex contention. ``Acquire``
    // pairs with the ``Release`` half of ``fetch_add(_, AcqRel)``
    // inside ``gpui_bump_generation``.
    let live_generation = state.current_generation.load(Ordering::Acquire);
    let mut slots = state.slots.lock().unwrap_or_else(|p| p.into_inner());
    match slots.remove(&token) {
        None => TAKE_UNKNOWN_TOKEN,
        Some(RenderSlot::Pending { generation }) => {
            if generation < live_generation {
                // ERV-M3: token was submitted before the most recent
                // story-switch. Consume it (do NOT re-insert) — the
                // worker will eventually publish Ready bytes against
                // this token, but those bytes are also stale; we
                // tolerate the orphan slot entry until the worker
                // fills it (we re-key by token, so future submits
                // get fresh tokens; this slot just overwrites with
                // stale Ready and is collected on next mismatch, or
                // never read at all).
                TAKE_STALE
            } else {
                // Re-insert so the next poll keeps the token alive.
                slots.insert(token, RenderSlot::Pending { generation });
                TAKE_PENDING
            }
        }
        Some(RenderSlot::Failed { generation, code }) => {
            if generation < live_generation {
                // Drop the stale failure too — the caller would
                // otherwise latch the async path off on a failure
                // that the bumped generation has already invalidated.
                TAKE_STALE
            } else {
                -(code as i32)
            }
        }
        Some(RenderSlot::Ready { generation, bytes }) => {
            if generation < live_generation {
                // Bytes are stale. ``bytes`` (a ``Vec<u8>``) drops
                // here, freeing the allocation. No leak.
                drop(bytes);
                TAKE_STALE
            } else {
                let len = bytes.len();
                let boxed: Box<[u8]> = bytes.into_boxed_slice();
                let ptr = Box::leak(boxed).as_mut_ptr();
                unsafe {
                    *out_ptr = ptr;
                    *out_len = len;
                }
                TAKE_READY
            }
        }
    }
}

/// Drop a pending or completed token without taking the bytes. Tests
/// and edge-case Nim cleanup paths use this to avoid leaking slot
/// entries when a connection terminates with renders in flight.
#[no_mangle]
pub extern "C" fn gpui_render_cancel(token: u32) {
    if token == 0 {
        return;
    }
    let state = WORKER.clone();
    let mut slots = state.slots.lock().unwrap_or_else(|p| p.into_inner());
    // ``remove`` drops whichever ``RenderSlot`` variant was stored —
    // ``Ready { bytes, .. }`` drops the ``Vec<u8>`` and frees the
    // allocation; ``Pending`` / ``Failed`` carry no heap data. No
    // leak on any branch.
    let _ = slots.remove(&token);
}
