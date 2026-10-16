//! Window management and event loop integration for the GPUI shim.
//!
//! This module provides:
//! - Window configuration (title, size, callbacks)
//! - A repaint flag for reactive update integration
//! - Lifecycle event callbacks (resize, focus, close)
//!
//! When the `gpui-backend` feature is enabled, `show_window` will launch an
//! actual GPUI window with event loop. Without the feature, the window state
//! is maintained in-memory for testing and headless operation.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

/// C function pointer types for lifecycle callbacks.
///
/// LEGACY PATH. None of these carries the window id, so the caller has
/// to hand a *distinct* function pointer per window — which on the Nim
/// side meant a fixed pool of pre-generated `cdecl` trampolines. See the
/// dispatcher types below for the replacement, and PLAT-19 /
/// `tests/test_window_callback_registry.nim` for what the pool cost.
pub type ResizeCallback = extern "C" fn(width: f64, height: f64);
pub type FocusCallback = extern "C" fn(focused: u8);
pub type CloseCallback = extern "C" fn() -> u8; // return 1 to allow close, 0 to prevent

/// PLAT-19 — window-id-carrying dispatchers.
///
/// One dispatcher is registered process-wide per event kind; every
/// window that opts in (`on_resize_dispatched` & co.) is delivered
/// through it, with its own id as the first argument. The consumer then
/// keeps an id-keyed registry and needs no per-window function pointer
/// at all — the same shape `gpui_set_event_dispatcher` +
/// `gpui_add_event_listener_id` already use for element events.
pub type WindowResizeDispatcher = extern "C" fn(window_id: u32, width: f64, height: f64);
pub type WindowFocusDispatcher = extern "C" fn(window_id: u32, focused: u8);
pub type WindowCloseDispatcher = extern "C" fn(window_id: u32) -> u8;

/// GWIN — called with the window's id when its chrome state
/// ([`ChromeState`]) changed between two frames. The consumer reads the new
/// values back through the `gpui_window_*` queries; passing them all as
/// arguments would make every future field an ABI break.
pub type WindowStateDispatcher = extern "C" fn(window_id: u32);

static RESIZE_DISPATCHER: Mutex<Option<WindowResizeDispatcher>> = Mutex::new(None);
static FOCUS_DISPATCHER: Mutex<Option<WindowFocusDispatcher>> = Mutex::new(None);
static CLOSE_DISPATCHER: Mutex<Option<WindowCloseDispatcher>> = Mutex::new(None);
static STATE_DISPATCHER: Mutex<Option<WindowStateDispatcher>> = Mutex::new(None);

fn take_lock<T>(m: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub fn set_resize_dispatcher(d: WindowResizeDispatcher) {
    *take_lock(&RESIZE_DISPATCHER) = Some(d);
}

pub fn set_focus_dispatcher(d: WindowFocusDispatcher) {
    *take_lock(&FOCUS_DISPATCHER) = Some(d);
}

pub fn set_close_dispatcher(d: WindowCloseDispatcher) {
    *take_lock(&CLOSE_DISPATCHER) = Some(d);
}

pub fn set_state_dispatcher(d: WindowStateDispatcher) {
    *take_lock(&STATE_DISPATCHER) = Some(d);
}

/// Unique window identifier (simple incrementing counter).
static NEXT_WINDOW_ID: AtomicU32 = AtomicU32::new(1);

/// Window lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowState {
    /// Window created but not yet shown.
    Created,
    /// Window is visible and the event loop is (or would be) running.
    Visible,
    /// Window close has been requested.
    CloseRequested,
    /// Window has been closed and destroyed.
    Closed,
}

/// Configuration and state for a single window.
pub struct WindowConfig {
    pub id: u32,
    pub title: String,
    pub width: f64,
    pub height: f64,
    pub state: WindowState,

    /// **PLAT-38: whether this window currently holds the seat's keyboard
    /// focus.** `notify_focus` used to forward the event and remember
    /// nothing, which was enough while focus was only something to report.
    /// It is not enough once keys are ROUTED: PLAT-38 asks for *"the same key
    /// delivered while the window does not hold focus changes nothing"*, and
    /// a shim with no memory of window focus cannot refuse.
    pub focused: bool,

    // Lifecycle callbacks (optional, LEGACY per-window function pointers)
    pub on_resize: Option<ResizeCallback>,
    pub on_focus: Option<FocusCallback>,
    pub on_close: Option<CloseCallback>,

    // PLAT-19: this window opted into the id-carrying global dispatcher
    // for the given event kind. Takes precedence over the legacy pointer.
    pub dispatch_resize: bool,
    pub dispatch_focus: bool,
    pub dispatch_close: bool,
    /// GWIN: this window opted into `gpui_on_window_state` delivery.
    pub dispatch_state: bool,

    /// GWIN: what the platform last said about this window's frame,
    /// sampled by `NimRootView::render` once per frame.
    pub chrome: ChromeState,
}

impl WindowConfig {
    pub fn new(title: &str, width: f64, height: f64) -> Self {
        WindowConfig {
            id: NEXT_WINDOW_ID.fetch_add(1, Ordering::Relaxed),
            title: title.to_string(),
            width,
            height,
            state: WindowState::Created,
            focused: false,
            on_resize: None,
            on_focus: None,
            on_close: None,
            dispatch_resize: false,
            dispatch_focus: false,
            dispatch_close: false,
            dispatch_state: false,
            chrome: ChromeState::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// GWIN — window chrome state
// ---------------------------------------------------------------------------
//
// The decoration mode, tiling, supported controls, maximized/active state and
// the desktop's button layout, as GPUI reports them for the live window. An
// application needs them to decide whether to draw a title bar and which
// buttons to put where (`isonim_gpui/window_environment`). The values are plain integers and a string so this module
// stays independent of the `gpui` crate and testable without a window.

/// `gpui_window_decorations`: the compositor draws the frame.
pub const DECORATIONS_SERVER: u8 = 0;
/// `gpui_window_decorations`: the application is expected to draw it.
pub const DECORATIONS_CLIENT: u8 = 1;

/// `gpui_window_tiling` bits (GPUI's `Tiling`).
pub const TILING_TOP: u8 = 1;
pub const TILING_RIGHT: u8 = 2;
pub const TILING_BOTTOM: u8 = 4;
pub const TILING_LEFT: u8 = 8;

/// `gpui_window_controls` bits (GPUI's `WindowControls`).
pub const CONTROL_MINIMIZE: u8 = 1;
pub const CONTROL_MAXIMIZE: u8 = 2;
pub const CONTROL_FULLSCREEN: u8 = 4;
pub const CONTROL_WINDOW_MENU: u8 = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChromeState {
    pub decorations: u8,
    pub tiling: u8,
    pub controls: u8,
    pub maximized: bool,
    pub active: bool,
    /// GNOME `button-layout` syntax (`"close:minimize,maximize"`): the
    /// buttons left of the colon go on the left. Empty when the platform
    /// reports no preference.
    pub button_layout: String,
}

impl Default for ChromeState {
    /// What is assumed before the first frame has been sampled: server-side
    /// decorations, and every control available, which is GPUI's own
    /// assumption (`WindowControls::default`, "we can do anything, unless
    /// told otherwise").
    fn default() -> Self {
        ChromeState {
            decorations: DECORATIONS_SERVER,
            tiling: 0,
            controls: CONTROL_MINIMIZE
                | CONTROL_MAXIMIZE
                | CONTROL_FULLSCREEN
                | CONTROL_WINDOW_MENU,
            maximized: false,
            active: false,
            button_layout: String::new(),
        }
    }
}

/// Format a button layout in GNOME's `button-layout` syntax, the one
/// `gtk-decoration-layout` uses and GPUI parses
/// (`WindowButtonLayout::parse`). The names are GPUI's `WindowButton::id`.
pub fn format_button_layout(left: &[&str], right: &[&str]) -> String {
    format!("{}:{}", left.join(","), right.join(","))
}

/// GPUI's own Linux fallback layout (`WindowButtonLayout::linux_default`)
/// in `format_button_layout` syntax.
pub const GPUI_FALLBACK_BUTTON_LAYOUT: &str = ":minimize,maximize,close";

/// The layout to REPORT as the desktop's, given what GPUI returned.
///
/// GPUI on Linux always answers `button_layout()`: it starts from its own
/// fallback and replaces it when the settings portal delivers the desktop's
/// `button-layout` (`gpui-pre-linux` `platform.rs:187`,
/// `xdg_desktop_portal.rs:100`). Its API cannot say which of the two it is
/// returning, and on a desktop with no settings portal the fallback is all
/// it will ever return. Measured on GNOME 2026-10-13: the first frame
/// reports the fallback, the next one the configured `:close`.
///
/// So a layout counts as the desktop's only when it is known to be: GPUI
/// announced a change (`reported`), or the value differs from the fallback.
/// Otherwise `""` — "no preference known" — and the helper falls back to
/// the platform convention and SAYS so (`psPlatformDefault`) instead of
/// passing a guess off as a configuration. A desktop configured to exactly
/// the fallback, whose portal answered before the window opened, therefore
/// reads as the default; the placement is the same either way.
pub fn desktop_button_layout(gpui_layout: String, reported: bool) -> String {
    if reported || gpui_layout != GPUI_FALLBACK_BUTTON_LAYOUT {
        gpui_layout
    } else {
        String::new()
    }
}

/// Opt `id` into `gpui_on_window_state` delivery.
pub fn enable_state_dispatch(id: u32) -> bool {
    with_window_mut(id, |w| w.dispatch_state = true).is_some()
}

/// The last sampled chrome state of window `id`.
pub fn chrome_state(id: u32) -> Option<ChromeState> {
    with_window(id, |w| w.chrome.clone())
}

/// Record a newly sampled chrome state. When it differs from the previous
/// one and the window opted in, the state dispatcher runs — after the
/// registry lock is released, so a handler may query the window it was
/// called for. Returns whether anything changed.
pub fn update_chrome_state(id: u32, state: ChromeState) -> bool {
    let Some((changed, dispatched)) = with_window_mut(id, |w| {
        let changed = w.chrome != state;
        w.chrome = state;
        (changed, w.dispatch_state)
    }) else {
        return false;
    };
    if changed && dispatched {
        if let Some(d) = *take_lock(&STATE_DISPATCHER) {
            d(id);
        }
    }
    changed
}

// ---------------------------------------------------------------------------
// GWIN — launch options, the run-time title, the launched window
// ---------------------------------------------------------------------------

/// Window properties an application sets BEFORE `gpui_launch`. They are
/// consumed (and reset) by the launch, so one launch's options never leak
/// into the next.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LaunchOptions {
    /// Wayland `app_id` / X11 `WM_CLASS`; should match the app's `.desktop`.
    pub app_id: Option<String>,
    /// The smallest content size the application can lay out.
    pub min_size: Option<(f64, f64)>,
    /// Open maximized; the launch size is then the restore size.
    pub maximized: bool,
}

static LAUNCH_OPTIONS: Mutex<LaunchOptions> = Mutex::new(LaunchOptions {
    app_id: None,
    min_size: None,
    maximized: false,
});

pub fn set_launch_app_id(app_id: &str) {
    take_lock(&LAUNCH_OPTIONS).app_id = if app_id.is_empty() {
        None
    } else {
        Some(app_id.to_string())
    };
}

/// Non-positive or non-finite dimensions clear the minimum.
pub fn set_launch_min_size(width: f64, height: f64) {
    let valid = width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0;
    take_lock(&LAUNCH_OPTIONS).min_size = if valid { Some((width, height)) } else { None };
}

pub fn set_launch_maximized(maximized: bool) {
    take_lock(&LAUNCH_OPTIONS).maximized = maximized;
}

/// Hand the options to the launch and reset them.
pub fn take_launch_options() -> LaunchOptions {
    std::mem::take(&mut *take_lock(&LAUNCH_OPTIONS))
}

/// The decoration mode requested at window open. GPUI asks for
/// server-side decorations by default and the compositor decides;
/// `ISONIM_GPUI_DECORATIONS=client|server` overrides the request (tests
/// use it to reach the client-side path on a compositor that would draw
/// the frame itself). Returns `Ok(None)` for "no override" and `Err` with
/// the offending value for anything else, which the caller reports and
/// ignores rather than guessing what was meant.
pub fn decorations_override(value: Option<&str>) -> Result<Option<u8>, String> {
    match value.map(str::trim) {
        None | Some("") => Ok(None),
        Some("client") => Ok(Some(DECORATIONS_CLIENT)),
        Some("server") => Ok(Some(DECORATIONS_SERVER)),
        Some(other) => Err(other.to_string()),
    }
}

/// A title set while the window is open, waiting for the next frame to
/// hand it to the platform (`Window::set_window_title` needs the window,
/// which only the render path holds).
static PENDING_TITLE: Mutex<Option<String>> = Mutex::new(None);

/// Change window `id`'s title. Returns false if there is no such window.
pub fn set_window_title(id: u32, title: &str) -> bool {
    let found = with_window_mut(id, |w| w.title = title.to_string()).is_some();
    if found {
        *take_lock(&PENDING_TITLE) = Some(title.to_string());
        request_repaint();
    }
    found
}

pub fn take_pending_title() -> Option<String> {
    take_lock(&PENDING_TITLE).take()
}

/// GWIN — where the application's content starts inside the platform
/// window: the frame's band on the left and top when the renderer draws the
/// decorations, (0, 0) otherwise. Written by the render path every frame;
/// read by `input::deliver_pointer`, which translates every pointer position
/// by it so the application only ever sees content coordinates. Two `f32`s
/// packed in one atomic so a reader never sees half of an update.
static CONTENT_ORIGIN: AtomicU64 = AtomicU64::new(0);

pub fn set_content_origin(x: f32, y: f32) {
    CONTENT_ORIGIN.store(
        ((x.to_bits() as u64) << 32) | y.to_bits() as u64,
        Ordering::Release,
    );
}

pub fn content_origin() -> (f32, f32) {
    let v = CONTENT_ORIGIN.load(Ordering::Acquire);
    (f32::from_bits((v >> 32) as u32), f32::from_bits(v as u32))
}

/// GWIN — whether the launched window is client-decorated this frame, the
/// condition under which drag regions act (`regions`). Written by the render
/// path every frame.
static CLIENT_DECORATED: AtomicBool = AtomicBool::new(false);

pub fn set_client_decorated(on: bool) {
    CLIENT_DECORATED.store(on, Ordering::Release);
}

pub fn client_decorated() -> bool {
    CLIENT_DECORATED.load(Ordering::Acquire)
}

/// GWIN — window operations requested from outside a title-bar press (a
/// menu entry, a key binding): latched here from any thread and carried out
/// on the next frame, where the platform window is in hand. Move and resize
/// are not among them: the compositor accepts those only inside the press
/// that starts them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowOp {
    Minimize,
    ToggleMaximize,
    ToggleFullscreen,
    Close,
}

static PENDING_OPS: Mutex<Vec<WindowOp>> = Mutex::new(Vec::new());

pub fn request_window_op(op: WindowOp) {
    take_lock(&PENDING_OPS).push(op);
    request_repaint();
}

/// The operations requested since the last frame, in request order.
pub fn take_window_ops() -> Vec<WindowOp> {
    std::mem::take(&mut *take_lock(&PENDING_OPS))
}

/// The decoration mode the launched window asked for (`decorations_override`
/// at launch): `None` when the compositor was left to choose. The render path
/// asks again once the window is mapped — see `NimRootView::sync_window_registry`.
static REQUESTED_DECORATIONS: AtomicU32 = AtomicU32::new(0);

pub fn set_requested_decorations(mode: Option<u8>) {
    REQUESTED_DECORATIONS.store(mode.map_or(0, |m| m as u32 + 1), Ordering::Release);
}

pub fn requested_decorations() -> Option<u8> {
    match REQUESTED_DECORATIONS.load(Ordering::Acquire) {
        0 => None,
        v => Some((v - 1) as u8),
    }
}

/// Font files an application registers before launching (`gpui_add_font_file`),
/// handed to GPUI's text system by every launch — windowed and headless — so
/// a family the application names in `font-family` is drawn in that face
/// rather than in the platform's fallback. Kept, not consumed: a process that
/// launches more than once registers them with each app.
static FONTS: Mutex<Vec<std::sync::Arc<Vec<u8>>>> = Mutex::new(Vec::new());

/// Read a font file (TrueType or OpenType) and keep it for every launch.
/// Errors: the file cannot be read, or it is empty.
pub fn add_font_file(path: &str) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    if bytes.is_empty() {
        return Err(format!("{path}: empty"));
    }
    take_lock(&FONTS).push(std::sync::Arc::new(bytes));
    Ok(())
}

/// The registered fonts' bytes, in registration order.
pub fn fonts() -> Vec<std::sync::Arc<Vec<u8>>> {
    take_lock(&FONTS).clone()
}

/// The registry id of the window `gpui_launch` is opening, from just before
/// it calls the application's root builder until the loop returns; 0 at
/// any other time. This is how an application registers resize and state
/// callbacks for the window it is launching.
pub static LAUNCH_WINDOW_ID: AtomicU32 = AtomicU32::new(0);

pub fn launch_window_id() -> u32 {
    LAUNCH_WINDOW_ID.load(Ordering::Acquire)
}

/// Global repaint flag. When set to true, the GPUI event loop (if running)
/// should trigger a redraw on the next frame via `cx.notify()`. Tree-mutating
/// functions set this flag, and the render loop clears it after processing.
pub static REPAINT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Request a repaint. This is called by tree mutation functions and can
/// also be called explicitly from the Nim side.
pub fn request_repaint() {
    REPAINT_REQUESTED.store(true, Ordering::Release);
}

/// Check and clear the repaint flag. Returns true if a repaint was requested.
pub fn take_repaint_request() -> bool {
    REPAINT_REQUESTED.swap(false, Ordering::AcqRel)
}

// ---------------------------------------------------------------------------
// Shutdown
// ---------------------------------------------------------------------------
//
// RS-M14b. `gpui_launch` under `gpui-backend` enters
// `Application::run(...)`, which does not return until the platform event
// loop stops. Before this existed the shim had NO way to stop it: the
// `gpui_close_window` / `gpui_reset_windows` pair only mutate the
// `WindowConfig` registry above, which the event loop never reads. So a
// process that called `gpui_launch` could only be killed, and the five
// launch cases in `tests/test_gui.nim` hung forever the moment the
// windowing backends were actually compiled in (measured 2026-09-17:
// nine render-plan cases `[OK]`, then nothing, rc=124 under a 90s cap).
//
// GPUI's own answer is `App::quit()` (crates/gpui/src/app.rs), which
// delegates to `Platform::quit()`; on Linux that is
// `common.signal.stop()` — the calloop `LoopSignal` — so `LinuxClient::run`
// returns and `Application::run` returns with it. What GPUI does NOT
// offer is a handle to reach `App` from outside the loop: `Rc<dyn
// Platform>` is `!Send` and `App` is only ever borrowed on the main
// thread inside an update. The two flags below are that missing handle.
// `launch_gpui_app` spawns one task inside the loop that reads them and
// calls `gpui_app::stop_platform_loop` from the place GPUI requires it to
// be called from.
//
// **AND `App::quit()` IS NOT THAT CALL ON EVERY PLATFORM.** The sentence
// above is a statement about LINUX. `MacPlatform::quit` is
// `-[NSApplication terminate:]`, which ends the process rather than
// unwinding, so on macOS the poller stops `-[NSApplication run]` itself;
// see `mac_event_loop.rs`. These two flags are unchanged by that and are
// what makes one poller serve both.

/// Set by `gpui_quit` (from any thread). The in-loop shutdown poller
/// installed by `launch_gpui_app` consumes it and quits the app.
pub static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Deadline, in milliseconds from the moment the event loop starts, after
/// which the app quits by itself. 0 disables it. Set by
/// `gpui_quit_after_ms` BEFORE `gpui_launch`, and reset to 0 once the
/// loop it armed has returned, so it can never leak into a later launch.
///
/// This is the backstop that makes a windowed test incapable of hanging
/// even when the thing that was supposed to request the quit never runs.
pub static AUTO_QUIT_MS: AtomicU32 = AtomicU32::new(0);

/// Request that a running GPUI event loop terminate. Thread-safe.
pub fn request_quit() {
    QUIT_REQUESTED.store(true, Ordering::Release);
}

/// Check and clear the quit flag.
pub fn take_quit_request() -> bool {
    QUIT_REQUESTED.swap(false, Ordering::AcqRel)
}

/// Observe the quit flag without consuming it.
pub fn quit_requested() -> bool {
    QUIT_REQUESTED.load(Ordering::Acquire)
}

/// Drop a pending quit request. Called at the top of `launch_gpui_app` so
/// a request left over from a previous loop cannot terminate the next one
/// before it has drawn anything.
pub fn clear_quit_request() {
    QUIT_REQUESTED.store(false, Ordering::Release);
}

/// Arm (ms > 0) or disarm (ms == 0) the auto-quit deadline.
pub fn set_auto_quit_ms(ms: u32) {
    AUTO_QUIT_MS.store(ms, Ordering::Release);
}

pub fn auto_quit_ms() -> u32 {
    AUTO_QUIT_MS.load(Ordering::Acquire)
}

/// The host's periodic callback: `(interval_ms, callback)`. Set by
/// `gpui_set_tick` before or during a loop; an interval of 0 (or no
/// callback) disarms it. Read by the loop's ticker on every period, so a
/// host can disarm it from inside its own callback.
pub type TickCallback = extern "C" fn();
static TICK: Mutex<Option<(u32, TickCallback)>> = Mutex::new(None);

/// Bumped by every `set_tick`: the loop's ticker watches it while it waits,
/// so a re-arm takes effect at once instead of after the wait in flight (a
/// host that armed a 5 s refresh and then needs a 300 ms deadline must not
/// wait out the 5 s).
static TICK_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Arm (`interval_ms > 0` and a callback) or disarm the host's tick. The
/// interval counts from THIS call: re-arming restarts the wait.
pub fn set_tick(interval_ms: u32, callback: Option<TickCallback>) {
    *take_lock(&TICK) = match callback {
        Some(cb) if interval_ms > 0 => Some((interval_ms, cb)),
        _ => None,
    };
    TICK_GENERATION.fetch_add(1, Ordering::AcqRel);
}

/// How many times the tick has been armed or disarmed.
pub fn tick_generation() -> u64 {
    TICK_GENERATION.load(Ordering::Acquire)
}

/// The armed tick, if any.
pub fn tick() -> Option<(u32, TickCallback)> {
    *take_lock(&TICK)
}

/// Global window registry. For now we support a single window (the common case).
/// The mutex protects concurrent access from the event loop thread and the Nim thread.
static WINDOWS: std::sync::LazyLock<Mutex<Vec<WindowConfig>>> =
    std::sync::LazyLock::new(|| Mutex::new(Vec::new()));

/// Lock the window registry.
fn lock_windows() -> std::sync::MutexGuard<'static, Vec<WindowConfig>> {
    match WINDOWS.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Create a new window configuration. Returns the window ID (> 0) or 0 on failure.
pub fn create_window(title: &str, width: f64, height: f64) -> u32 {
    let config = WindowConfig::new(title, width, height);
    let id = config.id;
    let mut windows = lock_windows();
    windows.push(config);
    id
}

/// Get a reference to a window by ID and apply a closure to it.
pub fn with_window<F, R>(id: u32, f: F) -> Option<R>
where
    F: FnOnce(&WindowConfig) -> R,
{
    let windows = lock_windows();
    windows.iter().find(|w| w.id == id).map(f)
}

/// Get a mutable reference to a window by ID and apply a closure to it.
pub fn with_window_mut<F, R>(id: u32, f: F) -> Option<R>
where
    F: FnOnce(&mut WindowConfig) -> R,
{
    let mut windows = lock_windows();
    windows.iter_mut().find(|w| w.id == id).map(f)
}

/// Show a window (transition to Visible state).
///
/// Without `gpui-backend`, this just updates the state.
/// With `gpui-backend`, this would launch the GPUI event loop.
///
/// Returns true if the state transition was valid.
pub fn show_window(id: u32) -> bool {
    with_window_mut(id, |w| {
        if w.state == WindowState::Created {
            w.state = WindowState::Visible;
            true
        } else {
            false
        }
    })
    .unwrap_or(false)
}

/// Request window close. Calls the on_close callback if registered.
/// Returns true if the window was closed (or close was accepted).
pub fn close_window(id: u32) -> bool {
    // Read the decision inputs under the lock, then call OUT of it — a
    // close handler is free to touch the window registry.
    let Some((dispatched, legacy)) = with_window(id, |w| (w.dispatch_close, w.on_close)) else {
        return false;
    };
    let dispatcher = *take_lock(&CLOSE_DISPATCHER);

    let allow_close = match (dispatched, dispatcher, legacy) {
        (true, Some(d), _) => d(id) != 0,
        (_, _, Some(cb)) => cb() != 0, // non-zero means allow close
        _ => true,                     // no handler means always allow
    };

    if allow_close {
        with_window_mut(id, |w| {
            w.state = WindowState::Closed;
            // A closed window holding keyboard focus would keep routing keys
            // to a surface that is gone — the negative twin passing for the
            // wrong reason, in the direction that looks like success.
            w.focused = false;
        });
    }

    allow_close
}

/// PLAT-19 — opt a window into id-carrying dispatch for one event kind.
/// Returns false if no window has that id.
pub fn enable_resize_dispatch(id: u32) -> bool {
    with_window_mut(id, |w| w.dispatch_resize = true).is_some()
}

pub fn enable_focus_dispatch(id: u32) -> bool {
    with_window_mut(id, |w| w.dispatch_focus = true).is_some()
}

pub fn enable_close_dispatch(id: u32) -> bool {
    with_window_mut(id, |w| w.dispatch_close = true).is_some()
}

/// Get the current state of a window. Returns None if window not found.
pub fn window_state(id: u32) -> Option<WindowState> {
    with_window(id, |w| w.state)
}

/// Get the current size of a window.
pub fn window_size(id: u32) -> Option<(f64, f64)> {
    with_window(id, |w| (w.width, w.height))
}

/// Simulate a resize event (for testing or when the real window resizes).
pub fn notify_resize(id: u32, width: f64, height: f64) {
    let Some((dispatched, legacy)) = with_window_mut(id, |w| {
        w.width = width;
        w.height = height;
        (w.dispatch_resize, w.on_resize)
    }) else {
        return;
    };
    if dispatched {
        if let Some(d) = *take_lock(&RESIZE_DISPATCHER) {
            d(id, width, height);
            return;
        }
    }
    if let Some(cb) = legacy {
        cb(width, height);
    }
}

/// Whether ANY window holds the seat's keyboard focus.
///
/// **THE ONE PREDICATE.** `input::gpui_dispatch_key_to_focus` refuses on it
/// and PLAT-38's negative-twin assertion reads it; two spellings would let
/// the control agree with itself while the rule was broken
/// (`Verification-Harness-Traps.md` §30).
pub fn any_window_focused() -> bool {
    lock_windows().iter().any(|w| w.focused)
}

/// Simulate a focus event.
pub fn notify_focus(id: u32, focused: bool) {
    // Recorded BEFORE the callback runs, so a handler that asks the shim
    // "am I focused?" from inside its own focus callback gets the state the
    // event announced rather than the one it replaced.
    with_window_mut(id, |w| w.focused = focused);
    let Some((dispatched, legacy)) = with_window(id, |w| (w.dispatch_focus, w.on_focus)) else {
        return;
    };
    if dispatched {
        if let Some(d) = *take_lock(&FOCUS_DISPATCHER) {
            d(id, if focused { 1 } else { 0 });
            return;
        }
    }
    if let Some(cb) = legacy {
        cb(if focused { 1 } else { 0 });
    }
}

/// Remove a window from the registry (cleanup).
pub fn destroy_window(id: u32) {
    let mut windows = lock_windows();
    windows.retain(|w| w.id != id);
}

/// Reset all windows (for testing).
pub fn reset_windows() {
    let mut windows = lock_windows();
    windows.clear();
    REPAINT_REQUESTED.store(false, Ordering::Release);
    // A quit request or an armed deadline that survived a reset would be
    // charged to whichever loop started next — a cross-test leak of
    // exactly the kind `reset_windows` exists to prevent.
    QUIT_REQUESTED.store(false, Ordering::Release);
    AUTO_QUIT_MS.store(0, Ordering::Release);
    set_tick(0, None);
    *take_lock(&LAUNCH_OPTIONS) = LaunchOptions::default();
    *take_lock(&PENDING_TITLE) = None;
    set_content_origin(0.0, 0.0);
    set_requested_decorations(None);
    set_client_decorated(false);
    take_lock(&PENDING_OPS).clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::sync::atomic::{AtomicU32 as TestAtomicU32, Ordering as TestOrdering};

    extern "C" fn noop_tick() {}

    #[test]
    #[serial]
    fn test_tick_arms_disarms_and_resets() {
        reset_windows();
        assert!(tick().is_none());
        set_tick(250, Some(noop_tick));
        assert_eq!(tick().map(|(ms, _)| ms), Some(250));
        // An interval of 0 disarms, even with a callback.
        set_tick(0, Some(noop_tick));
        assert!(tick().is_none());
        // No callback disarms, whatever the interval.
        set_tick(250, Some(noop_tick));
        set_tick(250, None);
        assert!(tick().is_none());
        // A reset leaves nothing armed for the next loop.
        set_tick(100, Some(noop_tick));
        reset_windows();
        assert!(tick().is_none());
    }

    #[test]
    #[serial]
    fn test_every_arming_moves_the_generation() {
        // The ticker restarts its wait when this moves: every arm and every
        // disarm must move it, or a re-arm would wait out the old interval.
        reset_windows();
        let g0 = tick_generation();
        set_tick(5000, Some(noop_tick));
        let g1 = tick_generation();
        assert!(g1 > g0);
        set_tick(300, Some(noop_tick));
        let g2 = tick_generation();
        assert!(g2 > g1);
        set_tick(0, None);
        assert!(tick_generation() > g2);
    }

    #[test]
    #[serial]
    fn test_create_window() {
        reset_windows();
        let id = create_window("Test", 800.0, 600.0);
        assert!(id > 0);
        assert_eq!(window_state(id), Some(WindowState::Created));
        assert_eq!(window_size(id), Some((800.0, 600.0)));
    }

    #[test]
    #[serial]
    fn test_show_window() {
        reset_windows();
        let id = create_window("Test", 800.0, 600.0);
        assert!(show_window(id));
        assert_eq!(window_state(id), Some(WindowState::Visible));
        // Cannot show again
        assert!(!show_window(id));
    }

    #[test]
    #[serial]
    fn test_close_window() {
        reset_windows();
        let id = create_window("Test", 800.0, 600.0);
        show_window(id);
        assert!(close_window(id));
        assert_eq!(window_state(id), Some(WindowState::Closed));
    }

    #[test]
    #[serial]
    fn test_close_callback_prevents_close() {
        reset_windows();
        let id = create_window("Test", 800.0, 600.0);
        show_window(id);

        extern "C" fn deny_close() -> u8 {
            0
        }
        with_window_mut(id, |w| {
            w.on_close = Some(deny_close);
        });

        assert!(!close_window(id));
        assert_eq!(window_state(id), Some(WindowState::Visible));
    }

    #[test]
    #[serial]
    fn test_close_callback_allows_close() {
        reset_windows();
        let id = create_window("Test", 800.0, 600.0);
        show_window(id);

        extern "C" fn allow_close() -> u8 {
            1
        }
        with_window_mut(id, |w| {
            w.on_close = Some(allow_close);
        });

        assert!(close_window(id));
        assert_eq!(window_state(id), Some(WindowState::Closed));
    }

    #[test]
    #[serial]
    fn test_resize_callback() {
        reset_windows();
        let id = create_window("Test", 800.0, 600.0);

        static RESIZE_W: TestAtomicU32 = TestAtomicU32::new(0);
        static RESIZE_H: TestAtomicU32 = TestAtomicU32::new(0);

        extern "C" fn on_resize(w: f64, h: f64) {
            RESIZE_W.store(w as u32, TestOrdering::SeqCst);
            RESIZE_H.store(h as u32, TestOrdering::SeqCst);
        }

        RESIZE_W.store(0, TestOrdering::SeqCst);
        RESIZE_H.store(0, TestOrdering::SeqCst);

        with_window_mut(id, |w| {
            w.on_resize = Some(on_resize);
        });

        notify_resize(id, 1024.0, 768.0);

        assert_eq!(RESIZE_W.load(TestOrdering::SeqCst), 1024);
        assert_eq!(RESIZE_H.load(TestOrdering::SeqCst), 768);
        assert_eq!(window_size(id), Some((1024.0, 768.0)));
    }

    #[test]
    #[serial]
    fn test_focus_callback() {
        reset_windows();
        let id = create_window("Test", 800.0, 600.0);

        static FOCUS_STATE: TestAtomicU32 = TestAtomicU32::new(99);

        extern "C" fn on_focus(focused: u8) {
            FOCUS_STATE.store(focused as u32, TestOrdering::SeqCst);
        }

        FOCUS_STATE.store(99, TestOrdering::SeqCst);

        with_window_mut(id, |w| {
            w.on_focus = Some(on_focus);
        });

        notify_focus(id, true);
        assert_eq!(FOCUS_STATE.load(TestOrdering::SeqCst), 1);

        notify_focus(id, false);
        assert_eq!(FOCUS_STATE.load(TestOrdering::SeqCst), 0);
    }

    #[test]
    #[serial]
    fn test_repaint_flag() {
        REPAINT_REQUESTED.store(false, Ordering::Release);
        assert!(!take_repaint_request());

        request_repaint();
        assert!(take_repaint_request());
        // Should be cleared after take
        assert!(!take_repaint_request());
    }

    // -----------------------------------------------------------------
    // RS-M14b — shutdown flags
    // -----------------------------------------------------------------
    //
    // These cover the FLAGS, in the default (non-GPUI) build, so the
    // semantics the in-loop poller depends on are checked by the 237-case
    // lane rather than only by the one job that needs a compositor. That
    // the poller then acts on them is a different claim, and the thing
    // that establishes it is the windowed pixel case in
    // `tests/test_gui.nim`: a process that returns from `gpui_launch`
    // could not have done so without `stop_platform_loop` having run.

    #[test]
    #[serial]
    fn test_quit_request_is_latched_and_consumed_once() {
        clear_quit_request();
        assert!(!quit_requested());
        assert!(!take_quit_request());

        request_quit();
        assert!(quit_requested(), "observing must not consume");
        assert!(quit_requested());
        assert!(take_quit_request());
        assert!(!take_quit_request(), "a taken request must not re-fire");
    }

    #[test]
    #[serial]
    fn test_auto_quit_deadline_round_trips_and_disarms() {
        set_auto_quit_ms(0);
        assert_eq!(auto_quit_ms(), 0, "0 is 'no deadline'");
        set_auto_quit_ms(2500);
        assert_eq!(auto_quit_ms(), 2500);
        set_auto_quit_ms(0);
        assert_eq!(auto_quit_ms(), 0);
    }

    #[test]
    #[serial]
    fn test_reset_windows_clears_shutdown_state() {
        // The negative control for the leak: without this, a test that
        // armed a quit would hand it to the next test's event loop.
        request_quit();
        set_auto_quit_ms(1234);
        reset_windows();
        assert!(!quit_requested());
        assert_eq!(auto_quit_ms(), 0);
    }

    #[test]
    #[serial]
    fn test_destroy_window() {
        reset_windows();
        let id = create_window("Test", 800.0, 600.0);
        assert!(window_state(id).is_some());
        destroy_window(id);
        assert!(window_state(id).is_none());
    }

    // -----------------------------------------------------------------
    // PLAT-19 — id-carrying dispatchers
    // -----------------------------------------------------------------

    static DISPATCH_LOG: Mutex<Vec<(u32, u64)>> = Mutex::new(Vec::new());

    fn log() -> std::sync::MutexGuard<'static, Vec<(u32, u64)>> {
        match DISPATCH_LOG.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    extern "C" fn resize_dispatcher(id: u32, w: f64, _h: f64) {
        log().push((id, w as u64));
    }

    extern "C" fn focus_dispatcher(id: u32, focused: u8) {
        log().push((id, focused as u64));
    }

    extern "C" fn close_dispatcher(id: u32) -> u8 {
        log().push((id, 0));
        // Deny the SECOND window created in the close test, allow others.
        u8::from(id % 2 == 1)
    }

    #[test]
    #[serial]
    fn test_dispatcher_routes_by_window_id() {
        reset_windows();
        log().clear();
        set_resize_dispatcher(resize_dispatcher);
        set_focus_dispatcher(focus_dispatcher);

        let a = create_window("A", 100.0, 100.0);
        let b = create_window("B", 100.0, 100.0);
        assert!(enable_resize_dispatch(a));
        assert!(enable_resize_dispatch(b));
        assert!(enable_focus_dispatch(a));
        assert!(enable_focus_dispatch(b));

        notify_resize(a, 11.0, 1.0);
        notify_resize(b, 22.0, 2.0);
        notify_focus(a, true);
        notify_focus(b, false);

        assert_eq!(*log(), vec![(a, 11), (b, 22), (a, 1), (b, 0)]);
    }

    #[test]
    #[serial]
    fn test_dispatcher_is_not_consulted_for_windows_that_did_not_opt_in() {
        // The negative control for the case above: a dispatcher that
        // fires for EVERY window would satisfy "each window's event
        // arrived" just as well, and only this can tell them apart.
        reset_windows();
        log().clear();
        set_resize_dispatcher(resize_dispatcher);

        let a = create_window("A", 100.0, 100.0);
        let b = create_window("B", 100.0, 100.0);
        assert!(enable_resize_dispatch(a));
        // b deliberately does NOT opt in and has no legacy callback.

        notify_resize(a, 11.0, 1.0);
        notify_resize(b, 22.0, 2.0);

        assert_eq!(*log(), vec![(a, 11)]);
    }

    #[test]
    #[serial]
    fn test_unknown_window_id_reaches_no_dispatcher() {
        reset_windows();
        log().clear();
        set_resize_dispatcher(resize_dispatcher);
        set_focus_dispatcher(focus_dispatcher);

        notify_resize(4242, 1.0, 1.0);
        notify_focus(4242, true);

        assert!(log().is_empty());
    }

    #[test]
    #[serial]
    fn test_close_dispatcher_decides_per_window() {
        reset_windows();
        log().clear();
        set_close_dispatcher(close_dispatcher);

        let a = create_window("A", 100.0, 100.0);
        let b = create_window("B", 100.0, 100.0);
        show_window(a);
        show_window(b);
        assert!(enable_close_dispatch(a));
        assert!(enable_close_dispatch(b));

        // `close_dispatcher` allows odd ids and denies even ones, so
        // exactly one of the two is refused whichever order they got.
        let a_closed = close_window(a);
        let b_closed = close_window(b);
        assert_eq!(a_closed, a % 2 == 1);
        assert_eq!(b_closed, b % 2 == 1);
        assert_ne!(
            a_closed, b_closed,
            "ids are consecutive, so exactly one closes"
        );
        assert_eq!(log().len(), 2);
    }

    #[test]
    #[serial]
    fn test_legacy_pointer_still_works_when_not_opted_in() {
        // The legacy `gpui_on_resize` path is a shipped API; the
        // dispatcher must be additive, not a replacement that silently
        // stops delivering to existing consumers.
        reset_windows();
        static LEGACY_W: TestAtomicU32 = TestAtomicU32::new(0);
        extern "C" fn legacy(w: f64, _h: f64) {
            LEGACY_W.store(w as u32, TestOrdering::SeqCst);
        }
        LEGACY_W.store(0, TestOrdering::SeqCst);

        let id = create_window("Legacy", 100.0, 100.0);
        with_window_mut(id, |w| w.on_resize = Some(legacy));
        notify_resize(id, 777.0, 1.0);
        assert_eq!(LEGACY_W.load(TestOrdering::SeqCst), 777);
    }

    #[test]
    #[serial]
    fn test_nonexistent_window() {
        reset_windows();
        assert_eq!(window_state(999), None);
        assert_eq!(window_size(999), None);
        assert!(!show_window(999));
        assert!(!close_window(999));
    }

    // ---- GWIN ------------------------------------------------

    static STATE_CALLS: TestAtomicU32 = TestAtomicU32::new(0);
    static STATE_LAST_ID: TestAtomicU32 = TestAtomicU32::new(0);
    extern "C" fn record_state(id: u32) {
        STATE_CALLS.fetch_add(1, TestOrdering::SeqCst);
        STATE_LAST_ID.store(id, TestOrdering::SeqCst);
        // The handler may query the window it was called for: the registry
        // lock is released before dispatch. A deadlock here hangs the test.
        let _ = chrome_state(id);
    }

    fn client_state() -> ChromeState {
        ChromeState {
            decorations: DECORATIONS_CLIENT,
            tiling: TILING_TOP | TILING_LEFT,
            controls: CONTROL_MAXIMIZE,
            maximized: true,
            active: true,
            button_layout: format_button_layout(&["close"], &[]),
        }
    }

    #[test]
    #[serial]
    fn test_chrome_state_dispatches_only_on_change_and_only_when_opted_in() {
        reset_windows();
        set_state_dispatcher(record_state);
        STATE_CALLS.store(0, TestOrdering::SeqCst);
        let a = create_window("A", 10.0, 10.0);
        let b = create_window("B", 10.0, 10.0);
        assert!(enable_state_dispatch(a));

        // Not opted in: recorded, not delivered.
        assert!(update_chrome_state(b, client_state()));
        assert_eq!(STATE_CALLS.load(TestOrdering::SeqCst), 0);
        assert_eq!(chrome_state(b), Some(client_state()));

        // Opted in: one delivery per change, none for a repeat.
        assert!(update_chrome_state(a, client_state()));
        assert!(!update_chrome_state(a, client_state()));
        assert_eq!(STATE_CALLS.load(TestOrdering::SeqCst), 1);
        assert_eq!(STATE_LAST_ID.load(TestOrdering::SeqCst), a);

        let mut restored = client_state();
        restored.maximized = false;
        assert!(update_chrome_state(a, restored));
        assert_eq!(STATE_CALLS.load(TestOrdering::SeqCst), 2);

        // Unknown windows change nothing and deliver nothing.
        assert!(!update_chrome_state(999, client_state()));
        assert!(!enable_state_dispatch(999));
        assert_eq!(STATE_CALLS.load(TestOrdering::SeqCst), 2);
    }

    #[test]
    #[serial]
    fn test_chrome_state_default_is_server_with_every_control() {
        reset_windows();
        let id = create_window("D", 10.0, 10.0);
        let s = chrome_state(id).unwrap();
        assert_eq!(s.decorations, DECORATIONS_SERVER);
        assert_eq!(s.controls, 15);
        assert_eq!(s.button_layout, "");
    }

    #[test]
    fn test_format_button_layout_is_gnome_syntax() {
        assert_eq!(
            format_button_layout(&[], &["minimize", "maximize", "close"]),
            ":minimize,maximize,close"
        );
        assert_eq!(
            format_button_layout(&["close"], &["minimize"]),
            "close:minimize"
        );
    }

    #[test]
    fn test_decorations_override_parses_and_refuses() {
        assert_eq!(decorations_override(None), Ok(None));
        assert_eq!(decorations_override(Some(" ")), Ok(None));
        assert_eq!(
            decorations_override(Some("client")),
            Ok(Some(DECORATIONS_CLIENT))
        );
        assert_eq!(
            decorations_override(Some("server")),
            Ok(Some(DECORATIONS_SERVER))
        );
        assert_eq!(decorations_override(Some("csd")), Err("csd".to_string()));
    }

    #[test]
    #[serial]
    fn test_launch_options_are_consumed_once() {
        reset_windows();
        set_launch_app_id("codetracer");
        set_launch_min_size(1050.0, 600.0);
        set_launch_maximized(true);
        let first = take_launch_options();
        assert_eq!(first.app_id.as_deref(), Some("codetracer"));
        assert_eq!(first.min_size, Some((1050.0, 600.0)));
        assert!(first.maximized);
        assert_eq!(take_launch_options(), LaunchOptions::default());

        // Invalid minimums clear rather than store garbage; "" clears the id.
        set_launch_min_size(-1.0, 600.0);
        set_launch_app_id("");
        assert_eq!(take_launch_options(), LaunchOptions::default());
        set_launch_min_size(f64::NAN, 1.0);
        assert_eq!(take_launch_options().min_size, None);
    }

    #[test]
    #[serial]
    fn test_window_title_is_recorded_and_latched() {
        reset_windows();
        let id = create_window("Old", 10.0, 10.0);
        assert!(set_window_title(id, "New"));
        assert_eq!(with_window(id, |w| w.title.clone()).as_deref(), Some("New"));
        assert_eq!(take_pending_title().as_deref(), Some("New"));
        assert_eq!(take_pending_title(), None);
        assert!(!set_window_title(999, "Nobody"));
        assert_eq!(take_pending_title(), None);
    }

    #[test]
    fn test_gpui_fallback_layout_is_not_reported_as_the_desktops() {
        let fallback = GPUI_FALLBACK_BUTTON_LAYOUT.to_string();
        assert_eq!(desktop_button_layout(fallback.clone(), false), "");
        assert_eq!(desktop_button_layout(fallback.clone(), true), fallback);
        assert_eq!(desktop_button_layout(":close".to_string(), false), ":close");
        assert_eq!(
            GPUI_FALLBACK_BUTTON_LAYOUT,
            format_button_layout(&[], &["minimize", "maximize", "close"])
        );
    }

    #[test]
    #[serial]
    fn test_content_origin_round_trips_and_resets() {
        reset_windows();
        assert_eq!(content_origin(), (0.0, 0.0));
        set_content_origin(10.0, 10.5);
        assert_eq!(content_origin(), (10.0, 10.5));
        set_requested_decorations(Some(DECORATIONS_CLIENT));
        assert_eq!(requested_decorations(), Some(DECORATIONS_CLIENT));
        set_requested_decorations(Some(DECORATIONS_SERVER));
        assert_eq!(requested_decorations(), Some(DECORATIONS_SERVER));
        reset_windows();
        assert_eq!(content_origin(), (0.0, 0.0));
        assert_eq!(requested_decorations(), None);
    }

    #[test]
    #[serial]
    fn test_window_ops_are_taken_in_order_once_and_reset() {
        reset_windows();
        request_window_op(WindowOp::Minimize);
        request_window_op(WindowOp::Close);
        assert!(
            take_repaint_request(),
            "a requested operation needs a frame"
        );
        assert_eq!(take_window_ops(), vec![WindowOp::Minimize, WindowOp::Close]);
        assert!(take_window_ops().is_empty());
        request_window_op(WindowOp::ToggleFullscreen);
        set_client_decorated(true);
        reset_windows();
        assert!(take_window_ops().is_empty());
        assert!(!client_decorated());
    }

    #[test]
    #[serial]
    fn test_font_files_are_kept_and_bad_ones_refused() {
        let before = fonts().len();
        let dir = std::env::temp_dir().join(format!("gwin-font-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("good.ttf");
        std::fs::write(&good, b"not really a font, but bytes").unwrap();
        let empty = dir.join("empty.ttf");
        std::fs::write(&empty, b"").unwrap();
        assert!(add_font_file(good.to_str().unwrap()).is_ok());
        assert!(add_font_file(empty.to_str().unwrap()).is_err());
        assert!(add_font_file(dir.join("missing.ttf").to_str().unwrap()).is_err());
        assert_eq!(fonts().len(), before + 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
