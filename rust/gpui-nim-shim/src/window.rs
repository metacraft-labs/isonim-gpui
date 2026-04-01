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
pub type ResizeCallback = extern "C" fn(width: f64, height: f64);
pub type FocusCallback = extern "C" fn(focused: u8);
pub type CloseCallback = extern "C" fn() -> u8; // return 1 to allow close, 0 to prevent

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

    // Lifecycle callbacks (optional)
    pub on_resize: Option<ResizeCallback>,
    pub on_focus: Option<FocusCallback>,
    pub on_close: Option<CloseCallback>,
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
    // First check the on_close callback
    let allow_close = with_window(id, |w| {
        if let Some(cb) = w.on_close {
            cb() != 0 // non-zero means allow close
        } else {
            true // no callback means always allow
        }
    })
    .unwrap_or(false);

    if allow_close {
        with_window_mut(id, |w| {
            w.state = WindowState::Closed;
        });
    }

    allow_close
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
    let callback = with_window_mut(id, |w| {
        w.width = width;
        w.height = height;
        w.on_resize
    });
    if let Some(Some(cb)) = callback {
        cb(width, height);
    }
}

/// Simulate a focus event.
pub fn notify_focus(id: u32, focused: bool) {
    let callback = with_window(id, |w| w.on_focus);
    if let Some(Some(cb)) = callback {
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
