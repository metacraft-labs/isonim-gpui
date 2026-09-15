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

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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

static RESIZE_DISPATCHER: Mutex<Option<WindowResizeDispatcher>> = Mutex::new(None);
static FOCUS_DISPATCHER: Mutex<Option<WindowFocusDispatcher>> = Mutex::new(None);
static CLOSE_DISPATCHER: Mutex<Option<WindowCloseDispatcher>> = Mutex::new(None);

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

    // Lifecycle callbacks (optional, LEGACY per-window function pointers)
    pub on_resize: Option<ResizeCallback>,
    pub on_focus: Option<FocusCallback>,
    pub on_close: Option<CloseCallback>,

    // PLAT-19: this window opted into the id-carrying global dispatcher
    // for the given event kind. Takes precedence over the legacy pointer.
    pub dispatch_resize: bool,
    pub dispatch_focus: bool,
    pub dispatch_close: bool,
}

impl WindowConfig {
    pub fn new(title: &str, width: f64, height: f64) -> Self {
        WindowConfig {
            id: NEXT_WINDOW_ID.fetch_add(1, Ordering::Relaxed),
            title: title.to_string(),
            width,
            height,
            state: WindowState::Created,
            on_resize: None,
            on_focus: None,
            on_close: None,
            dispatch_resize: false,
            dispatch_focus: false,
            dispatch_close: false,
        }
    }
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

/// Simulate a focus event.
pub fn notify_focus(id: u32, focused: bool) {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::sync::atomic::{AtomicU32 as TestAtomicU32, Ordering as TestOrdering};

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
}
