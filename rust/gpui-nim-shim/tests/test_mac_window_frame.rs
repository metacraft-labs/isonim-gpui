//! Regression test for `src/mac_window_frame.rs`: a `GPUIWindow` keeps a
//! frame larger than the display instead of being fitted to the screen's
//! visible frame.
//!
//! # The defect
//!
//! gpui's `GPUIWindow` inherits `-[NSWindow constrainFrameRect:toScreen:]`,
//! and AppKit's default for a titled window is to fit its frame to the
//! screen's `visibleFrame`. A 1920x1080-point request on a 1728x1117-point
//! display came back 1728x1084 — exactly the visible frame — where the
//! Electron front-end (Chromium overrides the method) got 1920x1080.
//! `install_unconstrained_window_frame` adds the override to `GPUIWindow`.
//!
//! # What this asserts, in order
//!
//! Every window is driven through gpui's own creation sequence, in
//! `MacWindow::open`'s order (`gpui-pre-macos-0.3.5/src/window.rs`):
//! `initWithContentRect:styleMask:backing:defer:screen:` (`:1067`), then
//! `orderFront:` (`:1292-1296`; gpui sends `makeKeyAndOrderFront:` when the
//! window takes focus, which orders it front the same way), then
//! `setFrameTopLeftPoint:` to the top of the screen (`:1302`). It is then
//! sent `setFrame:display:` while on screen, the path a later resize takes.
//! The frame is read after every step.
//!
//! 1. **Vacuity guard, stock `NSWindow`.** With gpui's style mask and a
//!    content rect 192 points wider and 96 points taller than the main
//!    screen's visible frame, it IS constrained: fitted into the visible
//!    frame by `orderFront:`, and altered by the on-screen
//!    `setFrame:display:`. This proves the display and the AppKit in hand
//!    exhibit the clamp; a pass of step 4 means nothing without it. If the
//!    clamp cannot be exhibited (no screen, no window server, or AppKit stops
//!    constraining), the test FAILS and says which; it never skips.
//! 2. **Vacuity guard, `GPUIWindow` before the install.** The same, on
//!    gpui's real class. This is the issue's observation reproduced
//!    in-process (1728x1084 on a 1728x1117-point display with a 33-point
//!    menu bar), and it shows that what step 4 measures is the override and
//!    not some property of `GPUIWindow` itself.
//! 3. `install_unconstrained_window_frame()` answers `Installed`.
//! 4. A second `GPUIWindow`, created after the install, keeps exactly the
//!    frame AppKit's own `frameRectForContentRect:` computes for the request
//!    after every step: same size and origin after `initWithContentRect:…`,
//!    `orderFront:` and `setFrame:display:`, and the same size with its top
//!    edge at the requested point after `setFrameTopLeftPoint:` — the
//!    proposed rect, unchanged, which is what the override returns.
//!
//! The frame after `initWithContentRect:…` is printed but not asserted on
//! the guards: on this host's AppKit it is not constrained there, because
//! the window is not yet on screen, and the guards need only show that the
//! on-screen paths constrain.
//!
//! Each window is on screen only between its `orderFront:` and its
//! `orderOut:`, so a run flashes three oversize windows.
//!
//! # No mocks
//!
//! Real AppKit, real window server, real gpui class: the `GPUIWindow` class
//! is the one `gpui-pre-macos`'s `#[ctor]` registers, looked up by name,
//! and the test refuses to run if it is absent rather than standing in a
//! look-alike subclass. Nothing is faked, so the test needs what AppKit
//! needs to create a window: a macOS GUI session with at least one screen.
//!
//! # Why `harness = false`
//!
//! AppKit requires windows to be created on the main thread, and libtest
//! runs every `#[test]` on a worker thread. This file is its own `main`
//! (declared `harness = false` in `Cargo.toml`), so it runs on the
//! process's main thread, and it exits non-zero when an assertion fails.
//! Off macOS it compiles to a `main` that says it has nothing to test —
//! there is no `GPUIWindow` and no `constrainFrameRect:toScreen:` there
//! (see `mac_window_frame.rs`'s *Platform guard*).
//!
//! # The `GPUIWindow` instances are leaked, deliberately
//!
//! gpui's `GPUIWindow` overrides `close` and `dealloc` to read its
//! `MacWindowState` ivar, which only gpui's own `MacWindow::open` sets. On
//! a bare instance that ivar is null, and either message would dereference
//! it. The instances are therefore ordered out, never closed and never
//! released: they are off screen and go away with the process. Ordering
//! one front and out is safe, because none of the methods gpui adds to the
//! class is reached on that path for a window that has no delegate. The
//! stock `NSWindow` control carries no such state and is closed normally.

#[cfg(target_os = "macos")]
mod macos {
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::{msg_send, MainThreadMarker};
    use objc2_app_kit::NSApplication;
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    use gpui_nim_shim::mac_window_frame::{install_unconstrained_window_frame, ConstrainOverride};

    /// gpui's style mask for `WindowKind::Normal`
    /// (`gpui-pre-macos-0.3.5/src/window.rs:992-1025`): titled | closable
    /// | miniaturizable | resizable. The clamp is a property of TITLED
    /// windows, so the mask is part of what is being reproduced.
    const STYLE_MASK: usize = 1 | 2 | 4 | 8;
    /// `NSBackingStoreBuffered`, as gpui passes it.
    const BACKING_BUFFERED: usize = 2;
    /// How far past the visible frame the request reaches. 192 is the
    /// issue's own number (1920 asked on a 1728-point display); the height
    /// overshoot only has to be large enough that the title bar does not
    /// decide the outcome.
    const EXTRA_WIDTH: f64 = 192.0;
    const EXTRA_HEIGHT: f64 = 96.0;

    /// A failed assertion, carried to `main` so it can print it and exit
    /// non-zero. `panic!` would do the same, but the explicit form keeps
    /// the message the only thing on stderr.
    type Check = Result<(), String>;

    fn rect(r: NSRect) -> String {
        format!(
            "{}x{} @ ({}, {})",
            r.size.width, r.size.height, r.origin.x, r.origin.y
        )
    }

    /// The frame AppKit would give a window with this style mask for
    /// `content`, unconstrained. Asked of the window itself, so the title
    /// bar height is AppKit's, not a constant here.
    fn frame_for_content(window: &AnyObject, content: NSRect) -> NSRect {
        unsafe { msg_send![window, frameRectForContentRect: content] }
    }

    fn frame(window: &AnyObject) -> NSRect {
        unsafe { msg_send![window, frame] }
    }

    /// `alloc` + gpui's own initialiser, on `screen`.
    fn new_window(class: &AnyClass, content: NSRect, screen: &AnyObject) -> Retained<AnyObject> {
        unsafe {
            let allocated: Allocated<AnyObject> = msg_send![class, alloc];
            let window: Option<Retained<AnyObject>> = msg_send![
                allocated,
                initWithContentRect: content,
                styleMask: STYLE_MASK,
                backing: BACKING_BUFFERED,
                defer: false,
                screen: screen
            ];
            window.unwrap_or_else(|| {
                panic!(
                    "-[{} initWithContentRect:…] returned nil",
                    class.name().to_string_lossy()
                )
            })
        }
    }

    /// The frames read along gpui's creation sequence and the resize path;
    /// see the module header for the order and the source lines.
    ///
    /// Measured on this host's AppKit (macOS 26.5.2) with the stock
    /// `NSWindow` control: `initWithContentRect:…` leaves the frame alone,
    /// because the window is not on screen yet; `orderFront:` fits the whole
    /// frame to the visible frame (1728x1084 here, the issue's number
    /// exactly); and `setFrame:display:` on the on-screen window keeps the
    /// title bar below the menu bar (height clamped, width not, here).
    #[derive(Clone, Copy)]
    struct Frames {
        /// AppKit's own unconstrained frame for the content rect.
        wanted: NSRect,
        /// `wanted`, moved so its top-left corner is at the point passed to
        /// `setFrameTopLeftPoint:`: what that message yields unconstrained.
        wanted_at_top_left: NSRect,
        after_init: NSRect,
        after_order_front: NSRect,
        after_top_left: NSRect,
        after_set_frame: NSRect,
    }

    const INIT: &str = "initWithContentRect:styleMask:backing:defer:screen:";
    const ORDER_FRONT: &str = "orderFront:";
    const TOP_LEFT: &str = "setFrameTopLeftPoint: (on screen)";
    const SET_FRAME: &str = "setFrame:display: (on screen)";

    /// Create a window of `class` and drive it through the sequence, then
    /// order it out again. The caller decides whether to close it.
    fn exercise(
        class: &AnyClass,
        content: NSRect,
        screen: &AnyObject,
        top_left: NSPoint,
    ) -> (Retained<AnyObject>, Frames) {
        let window = new_window(class, content, screen);
        let wanted = frame_for_content(&window, content);
        let wanted_at_top_left = NSRect::new(
            NSPoint::new(top_left.x, top_left.y - wanted.size.height),
            wanted.size,
        );
        let nil = std::ptr::null::<AnyObject>();

        let after_init = frame(&window);

        let () = unsafe { msg_send![&*window, orderFront: nil] };
        let after_order_front = frame(&window);

        let () = unsafe { msg_send![&*window, setFrameTopLeftPoint: top_left] };
        let after_top_left = frame(&window);

        let () = unsafe { msg_send![&*window, setFrame: wanted, display: false] };
        let after_set_frame = frame(&window);

        let () = unsafe { msg_send![&*window, orderOut: nil] };
        let frames = Frames {
            wanted,
            wanted_at_top_left,
            after_init,
            after_order_front,
            after_top_left,
            after_set_frame,
        };
        (window, frames)
    }

    fn print_frames(who: &str, f: &Frames) {
        for (path, got) in [
            (INIT, f.after_init),
            (ORDER_FRONT, f.after_order_front),
            (TOP_LEFT, f.after_top_left),
            (SET_FRAME, f.after_set_frame),
        ] {
            println!("{who} after {path}: {}", rect(got));
        }
    }

    fn same_size(a: NSRect, b: NSRect) -> bool {
        a.size.width == b.size.width && a.size.height == b.size.height
    }

    fn same_rect(a: NSRect, b: NSRect) -> bool {
        same_size(a, b) && a.origin.x == b.origin.x && a.origin.y == b.origin.y
    }

    /// The vacuity condition, shared by both guards: the on-screen paths
    /// constrain this window. `orderFront:` must show the issue's clamp
    /// (fitted INTO the visible frame), and the on-screen
    /// `setFrame:display:` must change the frame at all.
    fn require_constrained(who: &str, f: &Frames, visible: NSRect) -> Check {
        let fitted = f.after_order_front.size.width <= visible.size.width
            && f.after_order_front.size.height <= visible.size.height;
        if fitted && !same_size(f.after_set_frame, f.wanted) {
            return Ok(());
        }
        Err(format!(
            "VACUITY: {who} was not constrained as expected — after {ORDER_FRONT} {} (must fit \
             visibleFrame {}), after {SET_FRAME} {} (must differ from the unconstrained {}). \
             This AppKit or display does not exhibit the clamp, so a GPUIWindow keeping its \
             frame would prove nothing about the override.",
            rect(f.after_order_front),
            rect(visible),
            rect(f.after_set_frame),
            rect(f.wanted)
        ))
    }

    pub fn run() -> Check {
        let mtm = MainThreadMarker::new()
            .ok_or("not on the main thread: this file must be built with `harness = false`")?;
        // AppKit wants its application object before the first window.
        let _app = NSApplication::sharedApplication(mtm);

        let ns_screen = AnyClass::get(c"NSScreen").ok_or("no NSScreen class: AppKit not loaded")?;
        let screen: Option<Retained<AnyObject>> = unsafe { msg_send![ns_screen, mainScreen] };
        let screen = screen.ok_or(
            "VACUITY: +[NSScreen mainScreen] is nil — no window server or no display in this \
             session, so the clamp cannot be exhibited and this test cannot measure anything. \
             It needs a macOS GUI session; it does not skip.",
        )?;
        let visible: NSRect = unsafe { msg_send![&*screen, visibleFrame] };
        let screen_frame: NSRect = unsafe { msg_send![&*screen, frame] };
        println!("main screen frame:        {}", rect(screen_frame));
        println!("main screen visibleFrame: {}", rect(visible));

        let content = NSRect::new(
            visible.origin,
            NSSize::new(
                visible.size.width + EXTRA_WIDTH,
                visible.size.height + EXTRA_HEIGHT,
            ),
        );
        // gpui's `setFrameTopLeftPoint:` argument for a window at bounds
        // origin (0, 0): the top-left corner of the screen's FULL frame,
        // behind the menu bar (`window.rs:1055-1064`, `:1302`).
        let top_left = NSPoint::new(
            screen_frame.origin.x,
            screen_frame.origin.y + screen_frame.size.height,
        );
        println!("requested content rect:   {}", rect(content));

        let ns_window = AnyClass::get(c"NSWindow").ok_or("no NSWindow class")?;
        let gpui_window = AnyClass::get(c"GPUIWindow").ok_or(
            "no GPUIWindow class in this binary: gpui-pre-macos's #[ctor] class registration was \
             not linked, so there is nothing to test the override on",
        )?;

        // ---- 1. Vacuity guard: a stock NSWindow IS clamped ----------------
        let (control, c) = exercise(ns_window, content, &screen, top_left);
        let () = unsafe { msg_send![&*control, setReleasedWhenClosed: false] };
        let () = unsafe { msg_send![&*control, close] };
        drop(control);
        println!("unconstrained frame:      {}", rect(c.wanted));
        print_frames("control NSWindow", &c);
        require_constrained("a stock NSWindow", &c, visible)?;

        // ---- 2. Vacuity guard: GPUIWindow IS clamped without the override -
        let (before, b) = exercise(gpui_window, content, &screen, top_left);
        // Leaked: see the module header.
        std::mem::forget(before);
        print_frames("GPUIWindow before install", &b);
        require_constrained("a GPUIWindow before the install", &b, visible)?;

        // ---- 3. The override installs -------------------------------------
        let outcome = install_unconstrained_window_frame();
        println!("install_unconstrained_window_frame() = {outcome:?}");
        if outcome != ConstrainOverride::Installed {
            return Err(format!(
                "install_unconstrained_window_frame() = {outcome:?}, expected Installed"
            ));
        }

        // ---- 4. A GPUIWindow keeps the frame it asks for ------------------
        let (window, g) = exercise(gpui_window, content, &screen, top_left);
        // Leaked: see the module header.
        std::mem::forget(window);
        print_frames("GPUIWindow after install", &g);
        let mut failures = Vec::new();
        for (path, got, want) in [
            (INIT, g.after_init, g.wanted),
            (ORDER_FRONT, g.after_order_front, g.wanted),
            (TOP_LEFT, g.after_top_left, g.wanted_at_top_left),
            (SET_FRAME, g.after_set_frame, g.wanted),
        ] {
            if !same_rect(got, want) {
                failures.push(format!(
                    "GPUIWindow after {path}: frame {} is not the unconstrained {} \
                     (visibleFrame {}) — the window was fitted to the screen",
                    rect(got),
                    rect(want),
                    rect(visible)
                ));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("\n"))
        }
    }
}

fn main() {
    #[cfg(target_os = "macos")]
    match macos::run() {
        Ok(()) => println!("test_mac_window_frame: ok"),
        Err(message) => {
            eprintln!("test_mac_window_frame: FAILED\n{message}");
            std::process::exit(1);
        }
    }
    #[cfg(not(target_os = "macos"))]
    println!("test_mac_window_frame: nothing to test off macOS (no GPUIWindow class)");
}
