//! macOS: stopping the platform event loop so `gpui_launch` RETURNS.
//!
//! # Why this file exists
//!
//! `launch_gpui_app` is written against one contract, stated in its own
//! `# Shutdown` section and again in `spawn_shutdown_poller`'s header:
//! *"Three things end the loop, and all three take the same route out —
//! remove the windows, drain, quit"*, after which control comes back and
//! the function closes the window in the registry, disarms the deadline
//! and returns to its FFI caller.
//!
//! That contract holds on Linux and does not hold on macOS, and the
//! asymmetry is one function deep. `App::quit()` is
//! `self.platform.quit()`, and the two platform implementations do
//! opposite things with it:
//!
//! ```text
//! gpui-pre-linux-0.3.5/src/linux/platform.rs   fn quit() -> common.signal.stop()
//!                                              the calloop LoopSignal; `run` RETURNS
//! gpui-pre-macos-0.3.5/src/platform.rs:557     fn quit() -> [NSApplication terminate:]
//!                                              the process ENDS; `run` never returns
//! ```
//!
//! `-[NSApplication terminate:]` does not unwind. Everything after
//! `Application::run(...)` in `launch_gpui_app` is dead code on macOS, and
//! so is everything after `gpui_launch` in every consumer — measured as a
//! front-end that writes `--frame-report` and `--input-probe` after the
//! call and writes neither, while exiting 0. See
//! `issues/2026-09-29-gpui-launch-never-returns-on-macos.md`.
//!
//! # Which of the issue's three routes this is, and why
//!
//! Route 2: *"make the macOS quit path unwind instead of terminate"*. The
//! shim never reaches `Platform::quit` on macOS at all — `stop_event_loop`
//! below stops `-[NSApplication run]` directly, which is the loop
//! `MacPlatform::run` is blocked in, so `pool.drain()`, the ivar teardown,
//! `Application::run`'s return and `launch_gpui_app`'s post-loop block all
//! execute in order.
//!
//! Route 1 (move the post-loop work inside the loop) was rejected because
//! it repairs the shim's own invariants and leaves the ABI one-way: every
//! consumer with work after `gpui_launch` still loses it, which is the
//! actual reported defect. Route 3 (declare the one-way call) was rejected
//! as a FIX and kept as a TEST: `tests/test_gui.nim`'s completion sentinel
//! is route 3's second sentence, and it is not optional, because a suite
//! that stops in the middle and reports success is what made this
//! invisible to a Linux-only campaign — a failure mode that outlives this
//! particular cause.
//!
//! # Why here and not in a patched `gpui-pre-macos`
//!
//! The pin is a crates.io dependency (`gpui = { package = "gpui-pre",
//! version = "=0.3.5" }`), not a vendored tree. Patching it means a
//! `[patch.crates-io]` and a checked-in copy of the crate, which
//! `docs/gpui-pin.md`'s discipline exists to avoid: the pin should move
//! for a reason and be recorded when it does, and a local fork makes every
//! future bump a merge. The behaviour being worked around is also
//! upstream's deliberate one — Zed's own app DOES want `terminate:`,
//! because it is an application and not a library. It is the EMBEDDING
//! that is unusual here, so the embedder owns the workaround.
//!
//! What this costs, stated rather than hidden: `-[NSApplication stop:]`
//! plus a posted dummy event is AppKit behaviour that is documented but
//! not contracted by GPUI, so `docs/gpui-pin.md`'s *"re-read them on every
//! pin bump"* rule grows an entry.
//!
//! # The second macOS fact, which is NOT the same defect
//!
//! `-[NSApplication run]` sends `applicationDidFinishLaunching:` ONCE per
//! process, and `MacPlatform::run` parks the launch callback in
//! `state.finish_launching` for that notification to pick up
//! (`gpui-pre-macos-0.3.5/src/platform.rs:535`, taken at `:1350`). So a
//! SECOND `gpui_launch` in the same process would arm a callback nothing
//! ever calls: no window opens, no shutdown poller is spawned, and the
//! deadline that exists to stop exactly that cannot fire, because the task
//! that reads it was never created. That is a HANG, and it is a different
//! defect from the one above — fixing only the quit path converts the
//! first launch from "terminates the process" to "returns" and converts
//! the second from "never reached" to "never returns".
//!
//! `kick_relaunch` is what makes the second and later launches work: it
//! delivers `applicationDidFinishLaunching:` itself, from the main queue,
//! once the delegate `MacPlatform::run` installs is in place.

#![cfg(all(
    target_os = "macos",
    any(feature = "gpui-backend", feature = "gpui-headless")
))]

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{msg_send, sel, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventSubtype, NSEventType};
use objc2_foundation::NSPoint;

/// Stop `-[NSApplication run]` so `MacPlatform::run` unwinds.
///
/// Two steps, and the second is not optional. `-[NSApplication stop:]`
/// only sets a flag that `run`'s loop tests AFTER it has dispatched an
/// event; this is called from a main-queue block (the foreground executor
/// GPUI's `cx.spawn` tasks run on), which is a run-loop source and not an
/// event, so without something in the event queue the loop would go back
/// to waiting in `nextEventMatchingMask:` and the flag would not be read
/// until the user happened to move the mouse. The posted
/// `NSEventTypeApplicationDefined` event is that something.
///
/// Returns `false` in the two states where the stop did NOT land — called
/// off the main thread, or AppKit refused to make the wake event. The
/// first is a caller bug rather than a condition to recover from:
/// `App::quit` has the same requirement and `spawn_shutdown_poller`
/// already runs on the foreground executor for it. Either way the caller
/// says so out loud, because the symptom of a silent `false` is a process
/// that hangs until its lane's cap.
pub fn stop_event_loop() -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    app.stop(None);

    // A well-formed `NSEventTypeApplicationDefined` event with no graphics
    // context, which is the documented way to wake `run`'s event wait.
    // `postEvent:atStart:` retains it.
    let event =
        NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
            NSEventType::ApplicationDefined,
            NSPoint::ZERO,
            NSEventModifierFlags::empty(),
            0.0,
            0,
            None,
            NSEventSubtype::ApplicationActivated.0,
            0,
            0,
        );
    match event {
        Some(event) => {
            app.postEvent_atStart(&event, true);
            true
        }
        // An event the system refused to create would leave `stop:` armed
        // and unread. Say so rather than report a stop that did not land.
        None => false,
    }
}

/// How many times `launch_gpui_app` has entered the platform loop in this
/// process. Only "is this the first one" is ever read.
static LAUNCH_COUNT: AtomicU32 = AtomicU32::new(0);

/// Record that a launch is starting; answer whether it is a RE-launch and
/// therefore needs `kick_relaunch`.
pub fn begin_launch() -> bool {
    LAUNCH_COUNT.fetch_add(1, Ordering::AcqRel) > 0
}

/// Deliver `applicationDidFinishLaunching:` to the app delegate ourselves.
///
/// Queued on the main queue BEFORE `Application::run` is entered, so it
/// runs inside the event loop, after `MacPlatform::run` has installed the
/// delegate that handles it (`setDelegate:` precedes `app.run()` in that
/// function). gpui's handler ignores the notification argument entirely —
/// `extern "C" fn did_finish_launching(this: &mut Object, _: Sel, _: id)`
/// — so `nil` is passed rather than a fabricated `NSNotification`: a
/// notification nothing reads would be a prop.
pub fn kick_relaunch() {
    // SAFETY: `dispatch_async_f` with a `'static` extern "C" function and
    // a null context is the same shape of call `MacPlatform::quit` makes.
    unsafe {
        dispatch_async_f(dispatch_get_main_queue(), std::ptr::null_mut(), deliver);
    }

    extern "C" fn deliver(_: *mut c_void) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        let Some(delegate) = app.delegate() else {
            return;
        };
        if !delegate.respondsToSelector(sel!(applicationDidFinishLaunching:)) {
            return;
        }
        // SAFETY: the selector is declared on gpui's app-delegate class
        // and takes one object argument, which it does not dereference.
        unsafe {
            let _: () = msg_send![
                &*delegate,
                applicationDidFinishLaunching: std::ptr::null_mut::<AnyObject>(),
            ];
        }
    }
}

/// Stop the app delegate `MacPlatform::run` installed from observing
/// notifications, now that the loop it belonged to has ended.
///
/// **This closes a hazard `kick_relaunch` would otherwise open, and it is
/// stated rather than left implicit.** `did_finish_launching` registers
/// the delegate as an observer of
/// `NSTextInputContextKeyboardSelectionDidChangeNotification` and
/// `NSProcessInfoThermalStateDidChangeNotification` (and, conditionally,
/// the workspace's sleep/wake notifications). `MacPlatform::run` creates a
/// FRESH delegate on every call and nulls that delegate's platform ivar on
/// the way out, but never unregisters it — so in a process that launches
/// more than once, every previous delegate is still observing with a null
/// ivar, and `get_mac_platform`'s `assert!(!platform_ptr.is_null())` would
/// abort the process the next time the keyboard layout or the thermal
/// state changed. One launch per process never reached that state; five
/// (`tests/test_gui.nim`) would.
///
/// `removeObserver:` on an object that observes nothing is a no-op, so
/// this is safe on the first launch and on a launch that failed early.
pub fn release_delegate_observers() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let Some(delegate) = app.delegate() else {
        return;
    };
    let observer: &AnyObject = delegate.as_ref();
    // The DEFAULT centre only. `register_system_power_observers` puts the
    // delegate on `NSWorkspace`'s centre as well, but only when
    // `on_system_sleep` or `on_system_wake` has been installed, and this
    // shim installs neither — so reaching for `NSWorkspace` here would add
    // a dependency feature to remove an observation that is never made.
    let center = objc2_foundation::NSNotificationCenter::defaultCenter();
    unsafe { center.removeObserver(observer) };
}

// `libdispatch`'s two entry points, declared here rather than taken as a
// dependency: the `dispatch` crate is unmaintained and this is two symbols
// from `libSystem`, which is linked unconditionally on this platform.
#[repr(C)]
struct DispatchQueueS {
    _private: [u8; 0],
}

extern "C" {
    static _dispatch_main_q: DispatchQueueS;
    fn dispatch_async_f(
        queue: *mut DispatchQueueS,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
}

fn dispatch_get_main_queue() -> *mut DispatchQueueS {
    &raw const _dispatch_main_q as *mut DispatchQueueS
}
