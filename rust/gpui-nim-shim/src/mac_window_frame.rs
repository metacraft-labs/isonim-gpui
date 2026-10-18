//! macOS: letting a window keep the size it asked for.
//!
//! # The defect this closes
//!
//! `src/tests/visual/scenarios.json`'s `wide` viewport asks for a
//! 1920x1080-point window. On a display smaller than that the request is
//! not honoured: measured on this host (aarch64-darwin, macOS 26.5.2,
//! 1728x1117-point display), the GPUI window came out **1728x1084**
//! points — the full display width, and the display height less the
//! 33-point menu bar. Those two numbers are not a coincidence and not a
//! GPUI decision: 1728x1084 at the origin AppKit chose is exactly
//! `[[NSScreen mainScreen] visibleFrame]`.
//!
//! The Electron front-end, on the same display, gets the whole
//! 1920x1080 it asks for.
//!
//! The consequence is not a cosmetic difference. Three of the six
//! cross-renderer scenarios — `shell`, `editor` and `eventLog` — sit at
//! the `wide` viewport, and two captures with no extent in common cannot
//! be pixel-compared at all. Those three were unmeasurable for as long
//! as this stood.
//!
//! # The mechanism
//!
//! `-[NSWindow constrainFrameRect:toScreen:]` is AppKit's documented hook
//! for keeping a window inside the usable area of a screen. AppKit calls
//! it for a titled window on the paths gpui uses to place this window —
//! `initWithContentRect:styleMask:backing:defer:screen:` at creation
//! (`gpui-pre-macos-0.3.5/src/window.rs:1067`) and `setFrameTopLeftPoint:`
//! immediately after it (`:1299`) — and returns a rect shrunk and moved
//! to fit the screen's visible frame. Apple documents overriding it as
//! the supported way for a window to opt out.
//!
//! gpui does not override it. `build_window_class`
//! (`gpui-pre-macos-0.3.5/src/window.rs:435`) adds 27 methods to
//! `GPUIWindow` and `constrainFrameRect:toScreen:` is not among them, so
//! `GPUIWindow` inherits `NSWindow`'s constraining behaviour unmodified.
//! Nothing else in that crate clamps: its only other reference to a
//! visible frame is in the zoom path (`:870`), which this window never
//! takes.
//!
//! # What Chromium does, and why we follow it
//!
//! Chromium's window class for a native widget
//! (`NativeWidgetMacNSWindow`, in
//! `components/remote_cocoa/app_shim/native_widget_mac_nswindow.mm`)
//! overrides `constrainFrameRect:toScreen:` and returns the proposed rect
//! instead of calling `[super constrainFrameRect:toScreen:]`. The reason
//! is structural rather than stylistic: in a browser the window's size is
//! set by the layer above AppKit — the embedder, the window-placement
//! logic, or in our case a test harness naming a viewport — and that
//! layer, not the screen, is the authority on how big the window is. A
//! silently shrunk window makes the size the embedder was told it got a
//! lie.
//!
//! We follow it because the Electron front-end *is* Chromium, so this
//! override is the thing that makes the two renderers answer the same
//! way: it is the reason Electron gets its 1920x1080 on a 1728-point
//! display and GPUI did not. The project's rule for this campaign is to
//! recreate the Electron UI as precisely as possible in GPUI, which makes
//! the divergence a defect in GPUI rather than a property of the display
//! to be designed around. The alternative — shrinking the scenario to fit
//! this monitor — would make the suite pass by making it measure
//! something else, and would still differ from Electron on every display
//! smaller than the viewport.
//!
//! # Why a runtime override and not a patched crate
//!
//! The pin is a crates.io dependency (`gpui = { package = "gpui-pre",
//! version = "=0.3.5" }`) and `tools/check_gpui_pin.sh` keeps three
//! copies of that version in agreement. Patching it means a
//! `[patch.crates-io]` and a vendored copy of the crate, which
//! `docs/gpui-pin.md` exists to avoid. Adding the method to gpui's class
//! from here needs no fork, no vendored source and no new dependency:
//! `objc2` and `objc2-foundation` are already dependencies of this crate
//! and already used this way in `mac_event_loop.rs`.
//!
//! What it costs, stated rather than hidden: the Objective-C runtime is
//! not a GPUI API contract, so `docs/gpui-pin.md`'s *"re-read them on
//! every pin bump"* rule grows an entry. `install_unconstrained_window_frame`
//! reports which of the states below it actually reached, so a pin bump
//! that renames the class or starts defining the selector upstream is
//! visible in the launch log rather than silently inert.
//!
//! # Scope: `GPUIWindow` only, and that is a decision
//!
//! Adding a method to a class changes behaviour for **every instance of
//! that class**, so the scope is worth stating rather than discovering.
//!
//! gpui has two window classes, and this installs on one of them.
//! `WindowKind::Normal` allocates `GPUIWindow`; `PopUp`, `Floating` and
//! `Dialog` allocate `GPUIPanel`
//! (`gpui-pre-macos-0.3.5/src/window.rs:1012-1025`). Only `GPUIWindow` is
//! touched here, so menus, popovers, dialogs and floating panels keep
//! stock AppKit constraining and are still pulled back onto the screen
//! when they would fall off it — which is the behaviour a transient
//! surface should have, and which nothing in this campaign asks to
//! change.
//!
//! Within `GPUIWindow` the override is unconditional: it does not consult
//! the requested size or ask whether this particular window is the one a
//! test is measuring. A conditional override would be the more surgical
//! thing to write and the worse thing to own — the window is constrained
//! at creation, on every `setFrame:`, on a screen change and on a
//! resolution change, and a rule that fires on some of those paths and
//! not others is a window whose size depends on which path last touched
//! it. Chromium's override is unconditional for the same reason. The
//! practical blast radius is one window: `codetracer-gpui` opens a single
//! `WindowKind::Normal` window per launch.
//!
//! The visible consequence of the override is that a window larger than
//! the display now hangs off the edge of it instead of being shrunk. That
//! is what Electron already does here, and the capture path is unaffected
//! by it: `ci/test/plat37_macos_window_capture.nim` captures through
//! `SCContentFilter(desktopIndependentWindow:)` over the window's own
//! surface, not a crop of the display, so it reads the full window
//! whether or not all of it is on screen.
//!
//! # Platform guard
//!
//! This file carries its own `#![cfg(...)]` — `target_os = "macos"` AND one
//! of the two gpui features — the way `mac_event_loop.rs` does, so the
//! module is empty off macOS and the `mod` in `lib.rs` is unconditional.
//!
//! The Linux arm shares this renderer and is unaffected at three
//! independent levels, so this is a structural no-op there rather than a
//! behavioural one: the module compiles to nothing; its only call site
//! (`gpui_app.rs`) is itself `#[cfg(target_os = "macos")]`; and Linux does
//! not use this crate at all — `WindowKind` there is served by the separate
//! `gpui-pre-linux` crate, which contains no Objective-C class and no
//! `constrainFrameRect:toScreen:` (checked at 0.3.5: zero matches in its
//! `src`).

#![cfg(all(
    target_os = "macos",
    any(feature = "gpui-backend", feature = "gpui-headless")
))]

use std::ffi::CStr;
use std::sync::OnceLock;

use objc2::ffi::{class_addMethod, class_getInstanceMethod, method_getTypeEncoding};
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::sel;
use objc2_foundation::NSRect;

/// gpui's `WindowKind::Normal` class
/// (`gpui-pre-macos-0.3.5/src/window.rs:135`). Registered from a `#[ctor]`
/// in that crate, so it exists before `main` and this lookup cannot race
/// the registration.
const GPUI_WINDOW_CLASS: &CStr = c"GPUIWindow";

/// What `install_unconstrained_window_frame` did, so the caller can report
/// it rather than assume it.
///
/// Every variant but `Installed` means the window is still subject to
/// AppKit's constraining and the `wide` viewport will come back clamped.
/// They are distinguished because they fail for different reasons and a
/// pin bump is the likely cause of three of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstrainOverride {
    /// The method was added to `GPUIWindow`, overriding the one it
    /// inherited from `NSWindow`. This is the expected outcome.
    Installed,
    /// `GPUIWindow` already defined `constrainFrameRect:toScreen:` ITSELF
    /// — which 0.3.5 does not. Upstream has grown its own window-framing
    /// policy, and it is deliberately **left alone**: replacing a method
    /// gpui added would silently delete whatever it was there to do. If
    /// the clamp comes back, this is the variant to look for, and the
    /// remedy is to read the new upstream method and decide, not to
    /// force the replacement.
    DefinedUpstream,
    /// No class of that name. The class was renamed by a pin bump, or
    /// `gpui-pre-macos` was not linked into this binary at all.
    ClassMissing,
    /// `NSWindow` does not answer `constrainFrameRect:toScreen:`, so
    /// there is no type encoding to copy and nothing sound to install.
    /// Unreachable on any macOS that has the selector; reported rather
    /// than asserted because an abort in a launch path is worse than a
    /// clamped window.
    NoSuperclassMethod,
    /// The runtime refused the method. No known cause; reported so that
    /// "it silently did nothing" is not one of the possibilities.
    AddFailed,
}

impl ConstrainOverride {
    /// Whether the window will keep the frame it is given.
    pub fn is_installed(self) -> bool {
        self == ConstrainOverride::Installed
    }
}

/// The override: answer the proposed frame, unchanged.
///
/// This is the whole behavioural change. Returning `frame_rect` without
/// calling `super` is what Chromium's window class does and is the
/// documented way for an `NSWindow` subclass to decline the screen fit.
///
/// `screen` is ignored deliberately — the parameter is AppKit offering
/// the screen to fit to, and declining the fit is the point.
extern "C" fn constrain_frame_rect_to_screen(
    _this: &AnyObject,
    _cmd: Sel,
    frame_rect: NSRect,
    _screen: *mut AnyObject,
) -> NSRect {
    frame_rect
}

/// Memoised so a second `gpui_launch` in the same process reports what the
/// first one did instead of re-deriving it.
///
/// Without the memo the second call would read `class_addMethod`'s `NO`
/// — correct, the method is now present — and report `DefinedUpstream`,
/// which is a different and alarming claim. `tests/test_gui.nim` launches
/// five times in one process, so this is a path that is actually taken.
static OVERRIDE: OnceLock<ConstrainOverride> = OnceLock::new();

/// Add `constrainFrameRect:toScreen:` to gpui's `GPUIWindow`, so a window
/// keeps the frame it is given instead of being fitted to the screen.
///
/// Idempotent, and safe to call before any window exists — which is where
/// it must be called from, because the first constraining happens inside
/// `initWithContentRect:…`, before any pointer to the window is available
/// to the embedder. There is therefore no narrower, per-instance place to
/// put this: by the time an instance exists, its frame has already been
/// clamped once.
///
/// Returns what it did; see `ConstrainOverride`.
pub fn install_unconstrained_window_frame() -> ConstrainOverride {
    *OVERRIDE.get_or_init(install_once)
}

fn install_once() -> ConstrainOverride {
    let selector = sel!(constrainFrameRect:toScreen:);

    let Some(class) = AnyClass::get(GPUI_WINDOW_CLASS) else {
        return ConstrainOverride::ClassMissing;
    };

    // THE TYPE ENCODING IS COPIED FROM THE METHOD BEING OVERRIDDEN rather
    // than written out here. The literal for this signature is
    // `{CGRect={CGPoint=dd}{CGSize=dd}}@:{CGRect={CGPoint=dd}{CGSize=dd}}@`
    // on 64-bit Apple platforms, and a hand-written copy of it is a second
    // spelling of a fact the runtime already holds — wrong on any platform
    // where `CGFloat` is not a `double`, and wrong silently, because an
    // encoding mismatch is not a compile error but a garbled frame.
    //
    // `class_getInstanceMethod` searches the superclass chain, so at 0.3.5
    // this finds `NSWindow`'s own method: exactly the one we are replacing,
    // and therefore exactly the right encoding to register under.
    //
    // It doubles as a precondition. If AppKit has no such selector there is
    // nothing to override and we say so instead of installing a method
    // under a guessed signature.
    let superclass_method = unsafe { class_getInstanceMethod(class, selector) };
    if superclass_method.is_null() {
        return ConstrainOverride::NoSuperclassMethod;
    }
    let types = unsafe { method_getTypeEncoding(superclass_method) };
    if types.is_null() {
        return ConstrainOverride::NoSuperclassMethod;
    }

    // SAFETY: `selector`, the function and `types` agree — the function's
    // Rust signature is the C signature of
    // `-[NSWindow constrainFrameRect:toScreen:]` (`NSRect (id, SEL, NSRect,
    // id)`), and `types` is that method's own encoding, read above rather
    // than asserted here. `class` is a registered class and the cast to
    // `*mut` is what the runtime's own API takes; adding a method mutates
    // the class's dispatch table, which is a documented, thread-safe
    // runtime operation.
    //
    // `class_addMethod` DOES NOT replace: it returns false if the class
    // ITSELF already implements the selector, and ignores what it merely
    // inherits. That is the behaviour wanted here, and it is why
    // `class_replaceMethod` is not used — replacing would overwrite a
    // future upstream implementation without anyone noticing.
    let imp: Imp = unsafe {
        std::mem::transmute::<extern "C" fn(&AnyObject, Sel, NSRect, *mut AnyObject) -> NSRect, Imp>(
            constrain_frame_rect_to_screen,
        )
    };
    let added = unsafe {
        class_addMethod(
            class as *const AnyClass as *mut AnyClass,
            selector,
            imp,
            types,
        )
    };

    if added.as_bool() {
        ConstrainOverride::Installed
    } else {
        // False here means `GPUIWindow` carries its own implementation.
        // The memo above guarantees this is not simply our own second call.
        ConstrainOverride::DefinedUpstream
    }
}
