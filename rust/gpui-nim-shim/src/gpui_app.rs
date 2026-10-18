//! GPUI application lifecycle: launching the real GPUI window with the
//! shadow tree renderer as the root view.
//!
//! This module is only compiled when the `gpui-backend` feature is enabled.
//! It provides `launch_gpui_app()` which creates a GPUI window, registers
//! the shadow tree renderer view, and starts the event loop.
//!
//! The `NimRootView`, `render_plan_to_gpui`, and style/color helpers are
//! extracted as module-level items so integration tests can construct views
//! and verify rendering without launching a full event loop.

#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
use crate::render_sync::gpui_render::dispatch_shadow_event;
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
use crate::window;

// Import specific items from gpui rather than `use gpui::*` because gpui
// re-exports a `test` proc macro that shadows `#[test]` and causes infinite
// recursion in the compiler.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
use gpui::{
    div, img, px, rgb, rgba, size, AbsoluteLength, AnyElement, App, AppContext as _, Application,
    AsyncApp, Bounds, Context, Decorations, Div, FontWeight, HighlightStyle, Hsla,
    InteractiveElement, IntoElement, MouseButton, ParentElement, QuitMode, Render, Rgba, Styled,
    StyledText, Subscription, TitlebarOptions, WeakEntity, Window, WindowBounds, WindowDecorations,
    WindowOptions,
};

// RS-M14 Phase 2 (git pin): `Application::new()` from crates.io `gpui = "0.2"`
// was replaced by `Application::with_platform(Rc<dyn Platform>)` on the Zed
// monorepo `main` branch. `gpui_platform::current_platform` returns the OS-
// appropriate platform impl so we don't have to fan out `#[cfg]` ourselves.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
use gpui_platform::current_platform;

/// The ID of the window that is currently being displayed by GPUI.
/// Set before launching the event loop so that the root view and
/// event handlers can reference the correct window in the registry.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
static ACTIVE_WINDOW_ID: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);

/// Get the active GPUI window ID (0 if none).
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
#[allow(dead_code)]
pub fn active_window_id() -> u32 {
    ACTIVE_WINDOW_ID.load(std::sync::atomic::Ordering::Acquire)
}

// ---------------------------------------------------------------------------
// NimRootView: the GPUI view that reads the shadow tree and produces elements
// ---------------------------------------------------------------------------

/// NimRootView reads the global shadow tree and produces GPUI elements.
/// It is the root view for both the real application window and test windows.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
pub struct NimRootView {
    /// Whether the repaint polling timer has been started.
    poll_started: bool,

    /// **PLAT-38: the GPUI focus handle the root element tracks.**
    ///
    /// GPUI delivers key events only to the element tree that holds focus,
    /// so without a handle a window could paint perfectly and receive no key
    /// at all — which is the state `PLAT21-VG1` describes from the other
    /// side. It is created LAZILY on the first `render` rather than in
    /// `new()` because `new()` takes no context and is called from five
    /// places including two test files; a constructor that grew a parameter
    /// would put this milestone's diff across all of them for no gain.
    focus_handle: Option<gpui::FocusHandle>,

    /// GWIN: observers that re-render when the window's chrome state
    /// changes without a resize (the desktop's button layout, activation).
    /// Kept alive here; dropping a `Subscription` unregisters it.
    chrome_observers: Vec<Subscription>,

    /// GWIN: set once GPUI has told the window the desktop's button
    /// layout changed. See `desktop_button_layout`.
    layout_reported: std::rc::Rc<std::cell::Cell<bool>>,

    /// GWIN: the window's frame for the frame being built (see `frame.rs`).
    frame: crate::frame::FrameGeometry,

    /// GWIN: whether the launch's client-side decoration request has been
    /// repeated now that the window is mapped. Sway grants a client-side
    /// request only for a window that is floating when it asks (measured,
    /// sway 1.11), and GPUI asks once, at creation, before any window rule
    /// has floated it.
    decorations_rerequested: bool,

    /// GWIN: the launch size is the CONTENT's. GPUI opens a window whose
    /// whole size, band included, is the size it is given, so a window that
    /// is to draw its own decorations is created larger by the band (see
    /// `launch_gpui_app`), before the compositor has seen any size. When the
    /// compositor then grants server-side decorations instead, the first
    /// frame shrinks it back — still before the first commit. This holds the
    /// launch size; `None` once that first frame has run, or when nothing
    /// was added. It is pending until the decoration mode is known (see
    /// `sync_window_registry`).
    ///
    /// Correcting the size any later does not work: measured on headless
    /// GNOME Shell 49, a resize after the window was mapped changed GPUI's
    /// surface (660x420) while Mutter kept the geometry it had mapped
    /// (620x380), and the content overran the frame.
    pub launch_inflated: Option<(f32, f32)>,

    /// GWIN: the fallback title bar's height this frame — `regions::
    /// FALLBACK_BAR_HEIGHT` while the application draws the decorations, is
    /// not fullscreen and declares no drag region of its own; 0 otherwise.
    title_bar: f32,
}

#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
impl NimRootView {
    pub fn new() -> Self {
        NimRootView {
            poll_started: false,
            focus_handle: None,
            chrome_observers: Vec::new(),
            layout_reported: std::rc::Rc::new(std::cell::Cell::new(false)),
            frame: crate::frame::FrameGeometry::new(false, 0, false, 0.0, 0.0),
            decorations_rerequested: false,
            launch_inflated: None,
            title_bar: 0.0,
        }
    }

    /// GWIN — keep the window registry in step with the live
    /// platform window, before the frame is built so that whatever the
    /// application does in its callbacks is in THIS frame:
    ///
    ///   * a title set with `gpui_set_window_title` reaches the platform;
    ///   * the chrome state is sampled, and `gpui_on_window_state` fires if
    ///     it changed;
    ///   * a changed content size fires `gpui_on_resize`, so the
    ///     application lays itself out for the size it is about to be
    ///     drawn at. Until GWIN the shim never reported a resize at all.
    ///
    /// Only the launched window has a registry entry; the headless renderer
    /// (`ACTIVE_WINDOW_ID == 0`) skips all of it.
    fn sync_window_registry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let win_id = active_window_id();
        if win_id == 0 {
            return;
        }
        if self.chrome_observers.is_empty() {
            // GPUI does not refresh the window when the button layout
            // changes (`Window::button_layout_changed` only runs observers),
            // and activation changes need a frame to be sampled in.
            let reported = self.layout_reported.clone();
            self.chrome_observers
                .push(window.observe_button_layout_changed(move |window, _cx| {
                    reported.set(true);
                    window.refresh();
                }));
            self.chrome_observers
                .push(cx.observe_window_activation(window, |_view, _window, cx| cx.notify()));
            // A close the COMPOSITOR asks for (its own close button, a
            // keyboard shortcut) goes through the same predicate as the
            // application's close button and `gpui_window_request_close`:
            // the application's close callback decides.
            window.on_window_should_close(cx, move |_window, _cx| window::close_window(win_id));
        }
        for op in window::take_window_ops() {
            perform_window_op(op, window, win_id);
        }
        if let Some(title) = window::take_pending_title() {
            window.set_window_title(&title);
        }
        let chrome = sample_chrome_state(window, cx, self.layout_reported.get());
        // Pending until the decoration mode is known: GPUI's first frames
        // show its provisional client mode, and a compositor's answer comes a
        // frame or more later (measured on sway: one frame).
        if let Some((lw, lh)) = self.launch_inflated {
            if chrome.decorations == window::DECORATIONS_SERVER {
                // The band added at creation is not needed after all.
                self.launch_inflated = None;
                window.resize(size(px(lw), px(lh)));
            } else if chrome.active {
                // Mapped, focused and still client-decorated: the mode is
                // settled and the band is in use.
                self.launch_inflated = None;
            }
        }
        if !self.decorations_rerequested
            && window::requested_decorations() == Some(window::DECORATIONS_CLIENT)
            && chrome.decorations == window::DECORATIONS_SERVER
        {
            self.decorations_rerequested = true;
            window.request_decorations(WindowDecorations::Client);
        }

        // The frame for this frame: a band on every untiled side when the
        // application draws the decorations. GPUI is told the band so the
        // compositor's window geometry is the content, and pointer
        // positions are translated by the content's origin.
        let viewport = window.viewport_size();
        self.frame = crate::frame::FrameGeometry::new(
            chrome.decorations == window::DECORATIONS_CLIENT,
            chrome.tiling,
            chrome.maximized,
            f32::from(viewport.width),
            f32::from(viewport.height),
        );
        // The band GPUI reserves is constant under client-side decorations:
        // GPUI itself leaves it out on tiled edges (a maximized window is
        // tiled on all four), and it computes a restored window's geometry
        // from the inset in force at that moment. An inset dropped to 0 while
        // maximized made each maximize-restore cycle grow the window by the
        // band (measured on GNOME: 640x400 restored as 660x420).
        window.set_client_inset(px(
            if chrome.decorations == window::DECORATIONS_CLIENT {
                crate::frame::FRAME_INSET
            } else {
                0.0
            },
        ));
        window::set_client_decorated(chrome.decorations == window::DECORATIONS_CLIENT);
        let client = chrome.decorations == window::DECORATIONS_CLIENT;
        // The application hears about the new state FIRST: it may answer a
        // grant of client-side decorations by declaring its own title bar,
        // and the fallback bar is decided from the tree as it stands after
        // that. Decided before, an application that does so got one frame of
        // the fallback bar and two spurious resizes at every grant (measured
        // with CodeTracer on sway: 1280x800 -> 1260x748 -> 1280x800).
        window::update_chrome_state(win_id, chrome);
        let has_drag_region = {
            let tree = crate::lock_tree();
            let root = *crate::ROOT_NODE_ID.lock().unwrap_or_else(|p| p.into_inner());
            crate::regions::tree_declares_drag_region(&tree, root)
        };
        self.title_bar = if client && !window.is_fullscreen() && !has_drag_region {
            crate::regions::FALLBACK_BAR_HEIGHT
        } else {
            0.0
        };
        let (ox, oy) = self.frame.content_origin();
        window::set_content_origin(ox, oy + self.title_bar);

        // The application is told its CONTENT size: the window the
        // compositor reports, less the fallback title bar if one is drawn.
        let (cw, ch) = self.frame.content_size();
        let size = (cw as f64, (ch - self.title_bar).max(0.0) as f64);
        if window::window_size(win_id) != Some(size) {
            window::notify_resize(win_id, size.0, size.1);
        }
    }
}

/// Hand the application's registered font files (`window::add_font_file`) to
/// GPUI's text system. A file GPUI cannot parse is reported, not fatal: the
/// family it would have provided falls back to the platform's face.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
pub fn register_fonts(cx: &mut App) {
    let fonts = window::fonts();
    if fonts.is_empty() {
        return;
    }
    let owned: Vec<std::borrow::Cow<'static, [u8]>> = fonts
        .iter()
        .map(|f| std::borrow::Cow::Owned(f.as_ref().clone()))
        .collect();
    if let Err(e) = cx.text_system().add_fonts(owned) {
        eprintln!("gpui-nim-shim: registering {} font file(s) failed: {e}", fonts.len());
    }
    publish_font_report(cx);
}

/// Minimal JSON string escaping, for the font report's family names and file
/// paths. Both can contain a backslash or a quote on some hosts.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A family this font report is deliberately certain no host has.
///
/// It is the report's POSITIVE CONTROL and the reason the report can be read
/// as the RESOLVED face rather than the REQUESTED one. A probe that echoed
/// its input would answer this entry with monospace metrics identical to
/// `Space Mono`'s; a probe that asks the text system answers it with
/// whatever the fallback stack resolves to, whose `unitsPerEm` and advances
/// differ. `controlProvesResolution` below states that comparison as a
/// boolean so a reader does not have to make it by eye.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
const FONT_REPORT_CONTROL_FAMILY: &str = "ZZ No Such Face ZZ";

/// The families this report answers for: the two the CodeTracer front-end
/// names in `chrome.nim` (`MonoFontFamily`, `WindowFontFamily`), plus the
/// control.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
const FONT_REPORT_FAMILIES: [&str; 3] =
    ["Space Mono", "Space Grotesk", FONT_REPORT_CONTROL_FAMILY];

/// What the text system RESOLVED for each family this window draws in, as
/// JSON, published to `window::set_font_report` and — when
/// `GPUI_FONT_REPORT_PATH` names one — written to that file.
///
/// WHY METRICS AND NOT A NAME. GPUI's `TextSystem` hands back a `FontId` and
/// no way to ask it which file that id came from (`get_font_for_id` returns
/// the REQUESTED `Font` key, not the resolved face), so a name read back here
/// would be the request echoed. The resolved face's `unitsPerEm` and its
/// per-character advances are read through that id from the loaded face
/// itself, so they are properties of the FILE and not of the request: Space
/// Mono is 1000 units per em and 612/1000 em on every character, where the
/// platform fallbacks are 2048 units per em and proportional. That makes the
/// metrics a fingerprint of the face, which is also the only property of it
/// that the advance-drift question turns on.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
fn publish_font_report(cx: &mut App) {
    const SIZE_PX: f32 = 16.0;
    let text_system = cx.text_system().clone();
    let known: Vec<String> = text_system.all_font_names();

    let mut entries: Vec<String> = Vec::new();
    let mut fingerprints: Vec<Option<(u32, f32)>> = Vec::new();
    for family in FONT_REPORT_FAMILIES {
        // `resolve_font` panics only when neither the family nor any fallback
        // resolves. A diagnostic must never take the window down with it.
        let resolved = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let id = text_system.resolve_font(&gpui::font(family));
            let upem = text_system.units_per_em(id);
            let adv = |c: char| {
                text_system
                    .advance(id, gpui::px(SIZE_PX), c)
                    .map(|s| f32::from(s.width))
                    .unwrap_or(f32::NAN)
            };
            (upem, adv('M'), adv('i'), adv('W'), adv('0'))
        }));
        match resolved {
            Ok((upem, m, i, w, zero)) => {
                fingerprints.push(Some((upem, m)));
                entries.push(format!(
                    concat!(
                        "{{\"requested\": {}, \"knownToTextSystem\": {}, ",
                        "\"unitsPerEm\": {}, \"sizePx\": {}, ",
                        "\"advancePx\": {{\"M\": {}, \"i\": {}, \"W\": {}, \"0\": {}}}, ",
                        "\"monospaced\": {}, \"advanceEm\": {}}}"
                    ),
                    json_str(family),
                    known.iter().any(|n| n == family),
                    upem,
                    SIZE_PX,
                    m,
                    i,
                    w,
                    zero,
                    (m - i).abs() < 0.001 && (m - w).abs() < 0.001,
                    m / SIZE_PX
                ));
            }
            Err(_) => {
                fingerprints.push(None);
                entries.push(format!(
                    "{{\"requested\": {}, \"resolveFontPanicked\": true}}",
                    json_str(family)
                ));
            }
        }
    }

    // The control: the bogus family must NOT fingerprint like Space Mono.
    let control_proves_resolution = match (fingerprints.first(), fingerprints.last()) {
        (Some(Some((mono_upem, mono_adv))), Some(Some((ctl_upem, ctl_adv)))) => {
            mono_upem != ctl_upem || (mono_adv - ctl_adv).abs() > 0.001
        }
        _ => false,
    };

    let files: Vec<String> = window::font_paths().iter().map(|p| json_str(p)).collect();
    let json = format!(
        concat!(
            "{{\"probe\": \"gpui-nim-shim/register_fonts\", ",
            "\"textSystem\": \"gpui-pre 0.3.5 (platform text system)\", ",
            "\"registeredFileCount\": {}, \"registeredFiles\": [{}], ",
            "\"queries\": [{}], ",
            "\"controlFamily\": {}, \"controlProvesResolution\": {}, ",
            "\"controlMeaning\": \"true means the control family fingerprints ",
            "DIFFERENTLY from Space Mono, so these are resolved faces and not ",
            "the requested names echoed back\"}}"
        ),
        files.len(),
        files.join(", "),
        entries.join(", "),
        json_str(FONT_REPORT_CONTROL_FAMILY),
        control_proves_resolution
    );

    if let Ok(path) = std::env::var("GPUI_FONT_REPORT_PATH") {
        if !path.is_empty() {
            if let Err(e) = std::fs::write(&path, &json) {
                eprintln!("gpui-nim-shim: writing GPUI_FONT_REPORT_PATH {path}: {e}");
            }
        }
    }
    eprintln!("gpui-nim-shim: font-report {json}");
    window::set_font_report(json);
}

/// Read the platform's view of the window's frame (GWIN).
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
fn sample_chrome_state(window: &Window, cx: &App, layout_reported: bool) -> window::ChromeState {
    let (decorations, tiling) = match window.window_decorations() {
        Decorations::Server => (window::DECORATIONS_SERVER, 0),
        Decorations::Client { tiling } => {
            let mut bits = 0;
            if tiling.top {
                bits |= window::TILING_TOP;
            }
            if tiling.right {
                bits |= window::TILING_RIGHT;
            }
            if tiling.bottom {
                bits |= window::TILING_BOTTOM;
            }
            if tiling.left {
                bits |= window::TILING_LEFT;
            }
            (window::DECORATIONS_CLIENT, bits)
        }
    };
    let c = window.window_controls();
    let mut controls = 0;
    if c.minimize {
        controls |= window::CONTROL_MINIMIZE;
    }
    if c.maximize {
        controls |= window::CONTROL_MAXIMIZE;
    }
    if c.fullscreen {
        controls |= window::CONTROL_FULLSCREEN;
    }
    if c.window_menu {
        controls |= window::CONTROL_WINDOW_MENU;
    }
    let button_layout = cx
        .button_layout()
        .map(|layout| {
            let ids = |side: &[Option<gpui::WindowButton>]| -> Vec<&'static str> {
                side.iter().flatten().map(|b| b.id()).collect()
            };
            window::format_button_layout(&ids(&layout.left), &ids(&layout.right))
        })
        .map(|layout| window::desktop_button_layout(layout, layout_reported))
        .unwrap_or_default();
    window::ChromeState {
        decorations,
        tiling,
        controls,
        maximized: window.is_maximized(),
        active: window.is_window_active(),
        button_layout,
    }
}

#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
impl Render for NimRootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Check and clear the repaint flag (so we know the frame is current).
        let _ = window::take_repaint_request();

        // Start a background polling timer that checks REPAINT_REQUESTED
        // and calls cx.notify() to trigger re-renders. We only start this
        // once; the spawned task runs for the lifetime of the view.
        if !self.poll_started {
            self.poll_started = true;
            cx.spawn(async move |weak_entity: WeakEntity<NimRootView>, cx: &mut AsyncApp| {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(16))
                        .await;
                    if window::REPAINT_REQUESTED.load(std::sync::atomic::Ordering::Acquire) {
                        if let Some(entity) = weak_entity.upgrade() {
                            let _ = cx.update_entity(&entity, |_view: &mut NimRootView, cx: &mut Context<NimRootView>| {
                                cx.notify();
                            });
                        } else {
                            break; // entity was dropped
                        }
                    }
                }
            })
            .detach();
        }

        // PLAT-38 — THE KEYBOARD PATH, and it has three parts.
        //
        // 1. A focus handle exists and the root element TRACKS it. GPUI
        //    routes key events by focus, so without this the window paints
        //    and receives nothing.
        // 2. The window is told to focus it. A handle nothing focused is a
        //    handle no key reaches, and the failure looks exactly like "the
        //    key was never sent".
        // 3. The shim's WINDOW-FOCUS state is kept in step with GPUI's, so
        //    `input::deliver_key_to_focus` can refuse when the window does
        //    not hold focus. That refusal is PLAT-38's negative twin, and it
        //    is a real predicate rather than a constant precisely because
        //    this line can set it either way.
        let handle = self
            .focus_handle
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        if !handle.is_focused(window) {
            window.focus(&handle, cx);
        }
        window::notify_focus(active_window_id(), handle.is_focused(window));

        // PLAT-50 (CodeTracer): text a click copied goes to the platform
        // clipboard here, where an `App` is in hand (`clipboard.rs`).
        if let Some(text) = crate::clipboard::take_pending() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        }

        self.sync_window_registry(window, cx);

        let tree = crate::lock_tree();
        let root_id = *crate::ROOT_NODE_ID
            .lock()
            .unwrap_or_else(|p| p.into_inner());

        if root_id.is_null() {
            return with_keyboard(div().size_full().child("No shadow tree root"), &handle)
                .into_any_element();
        }

        let started = std::time::Instant::now();
        match crate::render_sync::build_render_plan(&tree, root_id) {
            Some(plan) => {
                drop(tree); // release lock before building GPUI elements
                let element = if self.frame.is_framed() || self.title_bar > 0.0 {
                    let title_bar = (self.title_bar > 0.0).then(|| {
                        let win_id = active_window_id();
                        let chrome = window::chrome_state(win_id).unwrap_or_default();
                        FallbackTitleBar {
                            height: self.title_bar,
                            title: window::with_window(win_id, |w| w.title.clone())
                                .unwrap_or_default(),
                            buttons: crate::regions::title_bar_buttons(
                                &chrome.button_layout,
                                chrome.controls,
                                cfg!(target_os = "macos"),
                            ),
                            maximized: chrome.maximized,
                        }
                    });
                    frame_client_window(
                        render_plan_to_gpui(&plan),
                        &self.frame,
                        title_bar,
                        FrameColors::of(&plan, window.is_window_active()),
                        &handle,
                    )
                } else {
                    render_root_to_gpui(&plan, &handle)
                };
                // PLAT-42: the render-path half of the frame budget. GPUI's
                // layout and paint follow and are not in this number.
                crate::frame_stats::complete_frame(started.elapsed().as_nanos() as u64);
                element
            }
            None => with_keyboard(div().size_full().child("Empty shadow tree"), &handle)
                .into_any_element(),
        }
    }
}

/// Attach the focus handle and the key listener to a root `Div`.
///
/// **ONE PLACE, and every root path goes through it** — the three fallback
/// roots above and the real render plan below. A second attachment site would
/// be a second chance for one of them to silently lose the keyboard, and the
/// symptom (a window that paints and never responds) is indistinguishable
/// from a compositor that sent nothing.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
fn with_keyboard(el: Div, handle: &gpui::FocusHandle) -> Div {
    el.track_focus(handle)
        // GWIN: every press starts unclaimed (`regions`); the capture phase
        // runs before any element's own handler.
        .capture_any_mouse_down(|_event, _window, _cx| crate::regions::begin_press())
        // A key context is what makes this element a keyboard dispatch
        // target in GPUI's tree. Named for this shim rather than for any
        // product, because the shim has no opinion about what the keys mean.
        .key_context("IsonimGpuiRoot")
        .on_key_down(move |event: &gpui::KeyDownEvent, _window, _cx| {
            let ks = &event.keystroke;
            // GPUI's own spelling — `key`, or, for a printable key typed with
            // no command modifier, GPUI's own `key_char`. See
            // `input::key_for_consumer` for why, and `input::modifier_bits`
            // for why the shim invents no name of its own.
            crate::input::deliver_key_to_focus(
                "keydown",
                Some((
                    crate::input::GPUI_EVENT_KEY_DOWN,
                    crate::input::key_for_consumer(
                        &ks.key,
                        ks.key_char.as_deref(),
                        ks.modifiers.control || ks.modifiers.alt || ks.modifiers.platform,
                    ),
                    crate::input::modifier_bits(&ks.modifiers),
                    event.is_held,
                )),
            );
        })
        // A modifier on its own never reaches `on_key_down`; its changes are
        // delivered as their own event, with the new set and no key.
        .on_modifiers_changed(move |event: &gpui::ModifiersChangedEvent, _window, _cx| {
            crate::input::deliver_key_to_focus(
                "modifierschange",
                Some((
                    crate::input::GPUI_EVENT_MODIFIERS,
                    String::new(),
                    crate::input::modifier_bits(&event.modifiers),
                    false,
                )),
            );
        })
}

/// Build the ROOT of the render plan, with the keyboard attached.
///
/// It shares `render_plan_to_gpui`'s Div arm through `build_plan_div` rather
/// than restating it; a root that was built by a second copy of that code
/// would drift from every non-root element in the same tree.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
pub fn render_root_to_gpui(
    plan: &crate::render_sync::RenderNode,
    handle: &gpui::FocusHandle,
) -> AnyElement {
    use crate::tree::GpuiElementKind;
    match plan.kind {
        GpuiElementKind::Div | GpuiElementKind::TextContainer => {
            with_keyboard(build_plan_div(plan), handle).into_any_element()
        }
        // An image, an SVG or a bare text node as the WHOLE root is not a
        // shape any product here produces; it is wrapped rather than
        // refused, and the wrapper is stated so a frame that gained a
        // container is explainable rather than surprising.
        _ => with_keyboard(div().size_full(), handle)
            .child(render_plan_to_gpui(plan))
            .into_any_element(),
    }
}

// ---------------------------------------------------------------------------
// GWIN: the client-side frame (geometry in `frame.rs`)
// ---------------------------------------------------------------------------

/// The frame's colours: the application's, from attributes on its root
/// element, or neutral defaults. A shadow is dimmed while the window is
/// inactive, as desktops do to show which window has focus.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
pub struct FrameColors {
    pub border: Hsla,
    pub shadow: Hsla,
    pub title_bar: Hsla,
    pub title_text: Hsla,
}

#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
impl FrameColors {
    pub fn of(plan: &crate::render_sync::RenderNode, active: bool) -> Self {
        let pick = |name: &str, default: &str| {
            plan.attributes
                .get(name)
                .and_then(|v| parse_color(v))
                .or_else(|| parse_color(default))
                .unwrap_or_default()
        };
        let mut shadow = pick("data-isonim-window-shadow", "#00000066");
        if !active {
            shadow.a *= 0.5;
        }
        FrameColors {
            border: pick("data-isonim-window-border", "#00000040"),
            shadow,
            title_bar: pick("data-isonim-titlebar-background", "#303030"),
            title_text: pick("data-isonim-titlebar-foreground", "#e6e6e6"),
        }
    }
}

/// Wrap the application's root in the client-side frame: a transparent
/// backdrop with a band on every untiled side, the content inside it with a
/// border and a shadow, and the resize zones on top.
///
/// The keyboard is attached to the backdrop, so the whole window remains the
/// key target. The resize zones occlude what is under them: a press there
/// resizes and does not reach the application.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
pub fn frame_client_window(
    content: AnyElement,
    frame: &crate::frame::FrameGeometry,
    title_bar: Option<FallbackTitleBar>,
    colors: FrameColors,
    handle: &gpui::FocusHandle,
) -> AnyElement {
    use crate::frame::{Edge, FRAME_BORDER};
    use gpui::{BoxShadow, CursorStyle, ResizeEdge};
    let (cw, ch) = frame.content_size();
    let side = |band: f32| if band > 0.0 { band - FRAME_BORDER } else { 0.0 };
    let border = |band: f32| if band > 0.0 { FRAME_BORDER } else { 0.0 };
    let body = div()
        .w(px(cw + border(frame.left) + border(frame.right)))
        .h(px(ch + border(frame.top) + border(frame.bottom)))
        .border_color(colors.border)
        .border_l(px(border(frame.left)))
        .border_t(px(border(frame.top)))
        .border_r(px(border(frame.right)))
        .border_b(px(border(frame.bottom)))
        .shadow(vec![BoxShadow::new(px(0.), px(0.), colors.shadow)
            .blur_radius(px(crate::frame::FRAME_INSET / 2.0))])
        .overflow_hidden()
        .flex()
        .flex_col();
    let bar_h = title_bar.as_ref().map_or(0.0, |b| b.height);
    let body = match title_bar {
        Some(bar) => body.child(fallback_title_bar(bar, cw, &colors)),
        None => body,
    }
    .child(div().w(px(cw)).h(px((ch - bar_h).max(0.0))).child(content));
    let mut backdrop = div()
        .size_full()
        .pl(px(side(frame.left)))
        .pt(px(side(frame.top)))
        .pr(px(side(frame.right)))
        .pb(px(side(frame.bottom)))
        .child(body);
    // GPUI hit-tests the LAST child on top, and `resize_zones` lists the
    // corners first (they win where they overlap an edge), so the zones are
    // added in reverse. Added in order, an edge's zone covered its corners:
    // a corner drag on sway resized along one axis only (measured).
    for (edge, zone) in frame.resize_zones().into_iter().rev() {
        let (cursor, resize) = match edge {
            Edge::Top => (CursorStyle::ResizeUpDown, ResizeEdge::Top),
            Edge::Bottom => (CursorStyle::ResizeUpDown, ResizeEdge::Bottom),
            Edge::Left => (CursorStyle::ResizeLeftRight, ResizeEdge::Left),
            Edge::Right => (CursorStyle::ResizeLeftRight, ResizeEdge::Right),
            Edge::TopLeft => (CursorStyle::ResizeUpLeftDownRight, ResizeEdge::TopLeft),
            Edge::BottomRight => (CursorStyle::ResizeUpLeftDownRight, ResizeEdge::BottomRight),
            Edge::TopRight => (CursorStyle::ResizeUpRightDownLeft, ResizeEdge::TopRight),
            Edge::BottomLeft => (CursorStyle::ResizeUpRightDownLeft, ResizeEdge::BottomLeft),
        };
        backdrop = backdrop.child(
            div()
                .absolute()
                .left(px(zone.x))
                .top(px(zone.y))
                .w(px(zone.w))
                .h(px(zone.h))
                .cursor(cursor)
                .occlude()
                .on_mouse_down(MouseButton::Left, move |_e, window, cx| {
                    cx.stop_propagation();
                    window.start_window_resize(resize);
                }),
        );
    }
    with_keyboard(backdrop, handle).into_any_element()
}

// ---------------------------------------------------------------------------
// Render plan -> GPUI element conversion (module-level for testability)
// ---------------------------------------------------------------------------

/// Recursively convert a RenderNode to GPUI AnyElement.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
pub fn render_plan_to_gpui(plan: &crate::render_sync::RenderNode) -> AnyElement {
    use crate::tree::GpuiElementKind;

    match plan.kind {
        GpuiElementKind::TextNode => {
            let text = plan.text.clone().unwrap_or_default();
            text.into_any_element()
        }
        GpuiElementKind::Img | GpuiElementKind::Svg
            if plan.attributes.get("src").map_or(false, |s| !s.is_empty()) =>
        {
            // A PICTURE FROM A FILE: `src` names a raster image or an SVG
            // document on disk, and GPUI's own `img()` loads it — a raster
            // format it recognises is decoded, anything else is rendered by
            // its SVG renderer (`ImageAssetLoader`). The element's `width`
            // and `height` size it, like any other box. Until this arm an
            // `img` was a grey placeholder with its alt text, so a front-end
            // could not draw an icon at all (CodeTracer's GPUI debugger
            // controls draw the desktop's SVG marks through it).
            let src = plan.attributes.get("src").cloned().unwrap_or_default();
            let mut el = img(std::path::PathBuf::from(src));
            if let Some(ref w) = plan.styles.w {
                if let Some(px_value) = parse_px(w) {
                    el = el.w(px(px_value));
                }
            }
            if let Some(ref h) = plan.styles.h {
                if let Some(px_value) = parse_px(h) {
                    el = el.h(px(px_value));
                }
            }
            el.into_any_element()
        }
        GpuiElementKind::Img => {
            // Placeholder: render a colored rect with alt text label.
            // Full image loading requires async fetching + GPUI's img() API.
            let label = plan.text.clone().unwrap_or_else(|| "[img]".to_string());
            let mut el = div();
            el = apply_styles_to_div(el, &plan.styles);
            // Give it a visible placeholder appearance if no explicit styles
            if plan.styles.bg.is_none() {
                el = el.bg(rgb(0xdddddd));
            }
            if plan.styles.w.is_none() {
                el = el.w(px(64.0));
            }
            if plan.styles.h.is_none() {
                el = el.h(px(64.0));
            }
            el = el.items_center().justify_center();
            el.child(label).into_any_element()
        }
        GpuiElementKind::Svg => {
            // Placeholder: render a colored rect with a label.
            // Full SVG rendering requires GPUI's svg() API with path data.
            let label = plan.text.clone().unwrap_or_else(|| "[svg]".to_string());
            let mut el = div();
            el = apply_styles_to_div(el, &plan.styles);
            // Give it a visible placeholder appearance if no explicit styles
            if plan.styles.bg.is_none() {
                el = el.bg(rgb(0xccccee));
            }
            if plan.styles.w.is_none() {
                el = el.w(px(64.0));
            }
            if plan.styles.h.is_none() {
                el = el.h(px(64.0));
            }
            el = el.items_center().justify_center();
            el.child(label).into_any_element()
        }
        GpuiElementKind::Div | GpuiElementKind::TextContainer => {
            build_plan_div(plan).into_any_element()
        }
    }
}

/// The `Div | TextContainer` arm's body, as a `Div` rather than an erased
/// `AnyElement`.
///
/// PLAT-38 split it out so the ROOT can attach a focus handle and a key
/// listener to the SAME element every other node is built as — see
/// `render_root_to_gpui`. Nothing about the body changed.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
pub fn build_plan_div(plan: &crate::render_sync::RenderNode) -> Div {
    let mut el = div();
    el = apply_styles_to_div(el, &plan.styles);

    // A DECLARED INLINE FORMATTING CONTEXT IS ONE TEXT ELEMENT, NOT ONE PER
    // SPAN (`render_sync::INLINE_TEXT_RUNS_ATTRIBUTE`, which carries the full
    // reasoning). GPUI rounds each text element's width up to a whole logical
    // pixel, so a row built from one element per coloured span accumulates
    // that rounding span by span and drifts right of where a browser puts the
    // same text. Fused, the row is rounded once.
    //
    // Everything below — events, pointer listeners, window regions — is
    // reached either way: only how the children are built changes.
    match fused_inline_text(plan) {
        Some(fused) => el = el.child(fused),
        None => {
            // Add children
            for child in &plan.children {
                el = el.child(render_plan_to_gpui(child));
            }

            // If the node has direct text content, add it as a child
            if let Some(ref text) = plan.text {
                if !text.is_empty() {
                    el = el.child(text.clone());
                }
            }
        }
    }

    // Wire click events
    if plan.has_click_handler {
        let node_id = crate::tree::NodeId(plan.node_id);
        el = el.on_mouse_up(MouseButton::Left, move |_event, _window, _cx| {
            dispatch_shadow_event(node_id, "click");
        });
    }

    el = wire_pointer_listeners(el, plan);
    wire_window_region(el, plan)
}

/// The single text element a declared inline formatting context collapses
/// to, or `None` when `plan` is not one (see `inline_text_runs`).
///
/// The per-span colours are handed over as HIGHLIGHTS rather than as finished
/// `TextRun`s. A `TextRun` must name a concrete `Font`, which this function
/// has no business choosing: the row's family, size and weight are inherited
/// from an ancestor and are known only once the window lays the element out.
/// `with_highlights` is resolved against `window.text_style()` at layout time
/// (`gpui-pre/src/elements/text.rs`), so the fused row keeps exactly the face
/// the unfused spans would have been drawn in, and only the colour is stated
/// here.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
fn fused_inline_text(plan: &crate::render_sync::RenderNode) -> Option<StyledText> {
    let (text, runs) = crate::render_sync::inline_text_runs(plan)?;
    let mut offset = 0usize;
    let mut highlights = Vec::with_capacity(runs.len());
    for run in &runs {
        let range = offset..offset + run.len;
        offset = range.end;
        // A span whose colour does not parse keeps the inherited colour,
        // which is what an unparsable `color` does to an unfused span too.
        let color = run.color.as_deref().and_then(parse_color);
        // `font-weight` is resolved by the SAME parser the unfused path
        // uses (`parse_font_weight`, applied at `el.font_weight(..)`), so a
        // run that fuses is drawn in the weight it would have had as its own
        // element. A span with no weight of its own leaves the field `None`
        // and inherits, exactly as it does unfused.
        let font_weight = run.font_weight.as_deref().map(parse_font_weight);
        highlights.push((
            range,
            HighlightStyle {
                color,
                font_weight,
                ..Default::default()
            },
        ));
    }
    Some(StyledText::new(text).with_highlights(highlights))
}

/// GWIN: give an element its window region (`regions.rs`). A drag region
/// moves, maximizes and shows the window menu; a window button acts on a
/// click; a `no-drag` element and any element listening for the pointer
/// claim the press so that no drag region behind them acts on it.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
fn wire_window_region(el: Div, plan: &crate::render_sync::RenderNode) -> Div {
    use crate::regions::{self, Region};
    let region = plan
        .attributes
        .get(regions::REGION_ATTRIBUTE)
        .and_then(|v| regions::parse_region(v));
    let claim = |el: Div| {
        el.on_mouse_down(MouseButton::Left, |_e, _w, _cx| regions::claim())
            .on_mouse_down(MouseButton::Right, |_e, _w, _cx| regions::claim())
    };
    match region {
        Some(Region::Drag) => drag_region(el),
        Some(button @ (Region::Minimize | Region::Maximize | Region::Close)) => {
            window_button(el, button)
        }
        Some(Region::NoDrag) => claim(el),
        None if regions::is_pointer_listener(plan.has_click_handler, &plan.event_names) => {
            claim(el)
        }
        None => el,
    }
}

/// A drag region's behaviour (`regions.rs`): a move on the first motion
/// after a left press, maximize on a double-click, the window menu on a
/// right press — unless an element in front claimed the press, or the
/// compositor draws the decorations.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
fn drag_region(el: Div) -> Div {
    use crate::regions;
    el.on_mouse_down(MouseButton::Left, |event, window, _cx| {
        if regions::claimed() || !window::client_decorated() {
            return;
        }
        // Exactly the second click: a quick third and fourth click must not
        // toggle maximize again (measured on GNOME: a double-click soon after
        // another arrived as counts 3 and 4, and maximized-then-restored in
        // one gesture).
        if event.click_count == 2 {
            regions::disarm_move();
            window.zoom_window();
        } else {
            regions::arm_move();
        }
    })
    .on_mouse_move(|event, window, _cx| {
        if event.pressed_button == Some(MouseButton::Left) && regions::take_move() {
            window.start_window_move();
        }
    })
    .on_mouse_up(MouseButton::Left, |_e, _w, _cx| regions::disarm_move())
    .on_mouse_down(MouseButton::Right, |event, window, _cx| {
        if !regions::claimed() && window::client_decorated() {
            window.show_window_menu(event.position);
        }
    })
}

/// A window button's behaviour: it claims the press and acts on the click.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
fn window_button(el: Div, button: crate::regions::Region) -> Div {
    use crate::regions::{self, Region};
    el.on_mouse_down(MouseButton::Left, |_e, _w, _cx| regions::claim())
        .on_mouse_down(MouseButton::Right, |_e, _w, _cx| regions::claim())
        .on_mouse_up(MouseButton::Left, move |_e, window, _cx| {
            let op = match button {
                Region::Minimize => window::WindowOp::Minimize,
                Region::Maximize => window::WindowOp::ToggleMaximize,
                _ => window::WindowOp::Close,
            };
            perform_window_op(op, window, active_window_id());
        })
}

/// The title bar the renderer draws for an application that declares none
/// of its own (GWIN): the window's title, and the buttons the platform
/// supports where the desktop wants them.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
pub struct FallbackTitleBar {
    pub height: f32,
    pub title: String,
    pub buttons: (Vec<crate::regions::Region>, Vec<crate::regions::Region>),
    pub maximized: bool,
}

/// The fallback title bar's button size, gap, and margin from the bar's ends.
pub const FALLBACK_BUTTON: f32 = 24.0;
pub const FALLBACK_BUTTON_GAP: f32 = 6.0;
pub const FALLBACK_BUTTON_MARGIN: f32 = 6.0;

#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
fn fallback_title_bar(bar: FallbackTitleBar, width: f32, colors: &FrameColors) -> Div {
    use crate::regions::Region;
    let fg = colors.title_text;
    let maximized = bar.maximized;
    let glyph = move |r: Region| -> AnyElement {
        match r {
            // Drawn rather than typed, so that no font has to carry them.
            Region::Minimize => div().w(px(10.)).h(px(1.5)).bg(fg).into_any_element(),
            Region::Maximize => {
                let side = if maximized { 8. } else { 10. };
                div()
                    .w(px(side))
                    .h(px(side))
                    .border_1()
                    .border_color(fg)
                    .into_any_element()
            }
            _ => div().text_color(fg).child("×").into_any_element(),
        }
    };
    let button = move |r: Region| {
        window_button(
            div()
                .w(px(FALLBACK_BUTTON))
                .h(px(FALLBACK_BUTTON))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(FALLBACK_BUTTON / 2.0))
                .bg(gpui::hsla(0., 0., 1., 0.08))
                .child(glyph(r)),
            r,
        )
    };
    let side = move |buttons: &[Region]| {
        div()
            .flex()
            .flex_row()
            .gap(px(FALLBACK_BUTTON_GAP))
            .children(buttons.iter().map(|r| button(*r)))
    };
    drag_region(
        div()
            .w(px(width))
            .h(px(bar.height))
            .flex_shrink_0()
            .flex()
            .flex_row()
            .items_center()
            .px(px(FALLBACK_BUTTON_MARGIN))
            .bg(colors.title_bar)
            .text_color(fg)
            .child(side(&bar.buttons.0))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .justify_center()
                    .overflow_hidden()
                    .child(bar.title),
            )
            .child(side(&bar.buttons.1)),
    )
}

/// Carry out a window operation on the platform window. `Close` asks the
/// application first (`window::close_window`), as a compositor close does.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
fn perform_window_op(op: window::WindowOp, window: &mut Window, win_id: u32) {
    match op {
        window::WindowOp::Minimize => window.minimize_window(),
        window::WindowOp::ToggleMaximize => window.zoom_window(),
        window::WindowOp::ToggleFullscreen => window.toggle_fullscreen(),
        window::WindowOp::Close => {
            if window::close_window(win_id) {
                window.remove_window();
            }
        }
    }
}

/// Wire the POINTER events a node listens for (`mousedown`, `mousemove`,
/// `mouseup`, `wheel`) to GPUI's mouse listeners, each delivering the
/// pointer's WINDOW position — what a consumer needs to hit-test a drag, a
/// divider or a drop zone against its own layout.
///
/// The position travels in the payload's `key` as `"x,y"` (logical window
/// pixels), and a wheel's as `"x,y,dx,dy"` (the delta in pixels, lines
/// converted at the window's line height), under the `GPUI_EVENT_POINTER_*`
/// kinds (`input.rs`). The payload's layout is unchanged, so every consumer
/// built against the key-only ABI still reads it.
///
/// Only the left button presses and releases under `mousedown` / `mouseup`: a
/// drag is a left-button gesture, and a listener for every button would hand a
/// right-click to a consumer that asked for a drag. The RIGHT button's press
/// is `contextmenu` and the MIDDLE button's `auxdown`, each under a kind of
/// its own (`GPUI_EVENT_POINTER_CONTEXT` / `_AUX`), for a consumer that asks.
/// A press carries the modifiers held. A move is reported whether or not a
/// button is down; the consumer knows whether a gesture is in flight.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
fn wire_pointer_listeners(mut el: Div, plan: &crate::render_sync::RenderNode) -> Div {
    let has = |name: &str| plan.event_names.iter().any(|n| n == name);
    let node_id = crate::tree::NodeId(plan.node_id);
    if has("mousedown") {
        el = el.on_mouse_down(MouseButton::Left, move |event, _window, _cx| {
            crate::input::deliver_pointer(
                node_id,
                "mousedown",
                crate::input::GPUI_EVENT_POINTER_DOWN,
                f32::from(event.position.x),
                f32::from(event.position.y),
                None,
                crate::input::modifier_bits(&event.modifiers),
            );
        });
    }
    if has("contextmenu") {
        el = el.on_mouse_down(MouseButton::Right, move |event, _window, _cx| {
            crate::input::deliver_pointer(
                node_id,
                "contextmenu",
                crate::input::GPUI_EVENT_POINTER_CONTEXT,
                f32::from(event.position.x),
                f32::from(event.position.y),
                None,
                crate::input::modifier_bits(&event.modifiers),
            );
        });
    }
    if has("auxdown") {
        el = el.on_mouse_down(MouseButton::Middle, move |event, _window, _cx| {
            crate::input::deliver_pointer(
                node_id,
                "auxdown",
                crate::input::GPUI_EVENT_POINTER_AUX,
                f32::from(event.position.x),
                f32::from(event.position.y),
                None,
                crate::input::modifier_bits(&event.modifiers),
            );
        });
    }
    if has("mousemove") {
        el = el.on_mouse_move(move |event, _window, _cx| {
            crate::input::deliver_pointer(
                node_id,
                "mousemove",
                crate::input::GPUI_EVENT_POINTER_MOVE,
                f32::from(event.position.x),
                f32::from(event.position.y),
                None,
                crate::input::modifier_bits(&event.modifiers),
            );
        });
    }
    if has("mouseup") {
        el = el.on_mouse_up(MouseButton::Left, move |event, _window, _cx| {
            crate::input::deliver_pointer(
                node_id,
                "mouseup",
                crate::input::GPUI_EVENT_POINTER_UP,
                f32::from(event.position.x),
                f32::from(event.position.y),
                None,
                crate::input::modifier_bits(&event.modifiers),
            );
        });
    }
    if has("wheel") {
        el = el.on_scroll_wheel(move |event, window, _cx| {
            let delta = event.delta.pixel_delta(window.line_height());
            crate::input::deliver_pointer(
                node_id,
                "wheel",
                crate::input::GPUI_EVENT_POINTER_WHEEL,
                f32::from(event.position.x),
                f32::from(event.position.y),
                Some((f32::from(delta.x), f32::from(delta.y))),
                crate::input::modifier_bits(&event.modifiers),
            );
        });
    }
    el
}

/// Apply GpuiStyles to a div builder.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
pub fn apply_styles_to_div(
    mut el: Div,
    styles: &crate::render_sync::GpuiStyles,
) -> Div {
    // Background color
    if let Some(ref bg) = styles.bg {
        if let Some(color) = parse_color(bg) {
            el = el.bg(color);
        }
    }

    // Width
    if let Some(ref w) = styles.w {
        if w == "100%" || w == "full" {
            el = el.w_full();
        } else if let Some(px_val) = parse_px(w) {
            el = el.w(px(px_val));
        }
    }

    // Height
    if let Some(ref h) = styles.h {
        if h == "100%" || h == "full" {
            el = el.h_full();
        } else if let Some(px_val) = parse_px(h) {
            el = el.h(px(px_val));
        }
    }

    // Flex direction
    if let Some(ref dir) = styles.flex_direction {
        match dir.as_str() {
            "row" => el = el.flex().flex_row(),
            "column" => el = el.flex().flex_col(),
            _ => {}
        }
    }

    // Padding
    if let Some(ref p) = styles.p {
        if let Some(px_val) = parse_px(p) {
            el = el.p(px(px_val));
        }
    }

    // Per-side padding, after `p` so a side named on its own wins
    // (2026-09-29: parsed into the plan as `padding_left` and never drawn).
    if let Some(v) = styles.padding_top.as_deref().and_then(parse_px) {
        el = el.pt(px(v));
    }
    if let Some(v) = styles.padding_right.as_deref().and_then(parse_px) {
        el = el.pr(px(v));
    }
    if let Some(v) = styles.padding_bottom.as_deref().and_then(parse_px) {
        el = el.pb(px(v));
    }
    if let Some(v) = styles.padding_left.as_deref().and_then(parse_px) {
        el = el.pl(px(v));
    }

    // Margin
    if let Some(ref m) = styles.m {
        if let Some(px_val) = parse_px(m) {
            el = el.m(px(px_val));
        }
    }
    // Per-side margins, after `m` so a side named on its own wins.
    if let Some(v) = styles.margin_top.as_deref().and_then(parse_px) {
        el = el.mt(px(v));
    }
    if let Some(v) = styles.margin_right.as_deref().and_then(parse_px) {
        el = el.mr(px(v));
    }
    if let Some(v) = styles.margin_bottom.as_deref().and_then(parse_px) {
        el = el.mb(px(v));
    }
    if let Some(v) = styles.margin_left.as_deref().and_then(parse_px) {
        el = el.ml(px(v));
    }

    // Gap
    if let Some(ref gap) = styles.gap {
        if let Some(px_val) = parse_px(gap) {
            el = el.gap(px(px_val));
        }
    }

    // Text color
    if let Some(ref tc) = styles.text_color {
        if let Some(color) = parse_color(tc) {
            el = el.text_color(color);
        }
    }

    // Border radius
    if let Some(ref r) = styles.rounded {
        if let Some(px_val) = parse_px(r) {
            el = el.rounded(px(px_val));
        }
    }
    // A corner of its own, over `rounded`: a tab joined to its panel is
    // rounded at the top only.
    if let Some(v) = styles.rounded_tl.as_deref().and_then(parse_px) {
        el = el.rounded_tl(px(v));
    }
    if let Some(v) = styles.rounded_tr.as_deref().and_then(parse_px) {
        el = el.rounded_tr(px(v));
    }
    if let Some(v) = styles.rounded_br.as_deref().and_then(parse_px) {
        el = el.rounded_br(px(v));
    }
    if let Some(v) = styles.rounded_bl.as_deref().and_then(parse_px) {
        el = el.rounded_bl(px(v));
    }

    // Align items
    if let Some(ref items) = styles.items {
        match items.as_str() {
            "center" => el = el.items_center(),
            "start" => el = el.items_start(),
            "end" => el = el.items_end(),
            _ => {}
        }
    }

    // Justify content
    if let Some(ref justify) = styles.justify {
        match justify.as_str() {
            "center" => el = el.justify_center(),
            "start" => el = el.justify_start(),
            "end" => el = el.justify_end(),
            "space_between" => el = el.justify_between(),
            "space_around" => el = el.justify_around(),
            _ => {}
        }
    }

    // Cursor
    if let Some(ref cursor) = styles.cursor {
        if cursor == "pointer" {
            el = el.cursor_pointer();
        }
    }

    // THE STYLES BELOW WERE PARSED INTO THE PLAN AND NEVER DRAWN until
    // 2026-09-23. The render plan (`gpui_render_plan_json`) reported them, so a
    // suite reading the plan saw a flex row and a dimmed line while the WINDOW
    // stacked the row's children vertically and dimmed nothing. A window frame
    // compared against the plan is what found it; the two must agree, so every
    // style the plan carries that has a GPUI equivalent is applied here.

    // `display: flex` with no direction is CSS's default, a ROW.
    if let Some(ref display) = styles.display {
        if display.trim() == "flex" && styles.flex_direction.is_none() {
            el = el.flex().flex_row();
        }
    }

    // Opacity, a number in [0, 1]; anything else is ignored rather than
    // guessed at.
    if let Some(ref opacity) = styles.opacity {
        if let Ok(v) = opacity.trim().parse::<f32>() {
            if (0.0..=1.0).contains(&v) {
                el = el.opacity(v);
            }
        }
    }

    if let Some(ref overflow) = styles.overflow {
        if overflow.trim() == "hidden" {
            el = el.overflow_hidden();
        }
    }

    // `white-space: nowrap` / `pre` keep a line on one row; `normal` wraps.
    if let Some(ref ws) = styles.white_space {
        match ws.trim() {
            "nowrap" | "pre" => el = el.whitespace_nowrap(),
            "normal" => el = el.whitespace_normal(),
            _ => {}
        }
    }

    if let Some(ref to) = styles.text_overflow {
        if to.trim() == "ellipsis" {
            el = el.text_ellipsis();
        }
    }

    if let Some(ref shrink) = styles.flex_shrink {
        if shrink.trim() == "0" {
            el = el.flex_shrink_0();
        }
    }

    // `flex-grow: N`, a non-negative grow factor: the element takes its
    // share of the free space on its parent's main axis.
    if let Some(ref grow) = styles.flex_grow {
        if let Ok(g) = grow.trim().parse::<f32>() {
            if g >= 0.0 {
                el = el.flex_grow(g);
            }
        }
    }

    // min-width, as a pixel value — what lets a flex child shrink below its
    // content (`min-width: 0`) so an ellipsis can apply.
    if let Some(ref mw) = styles.min_w {
        if let Some(px_val) = parse_px(mw) {
            el = el.min_w(px(px_val));
        }
    }

    // BORDERS AND WEIGHT (2026-09-29). Both were parsed into the plan and
    // never drawn — the plan said a pane was outlined and a tab was bold, the
    // window showed neither. A width applies to every side unless a per-side
    // width names that side; a width without a colour draws in GPUI's
    // default border colour.
    let all_sides = styles.border_width.as_deref().and_then(parse_px);
    let side = |s: &Option<String>| s.as_deref().and_then(parse_px).or(all_sides);
    let widths = [
        side(&styles.border_top_width),
        side(&styles.border_right_width),
        side(&styles.border_bottom_width),
        side(&styles.border_left_width),
    ];
    if widths.iter().any(|w| w.is_some()) {
        let st = el.style();
        if let Some(v) = widths[0] {
            st.border_widths.top = Some(AbsoluteLength::Pixels(px(v)));
        }
        if let Some(v) = widths[1] {
            st.border_widths.right = Some(AbsoluteLength::Pixels(px(v)));
        }
        if let Some(v) = widths[2] {
            st.border_widths.bottom = Some(AbsoluteLength::Pixels(px(v)));
        }
        if let Some(v) = widths[3] {
            st.border_widths.left = Some(AbsoluteLength::Pixels(px(v)));
        }
    }
    if let Some(color) = styles.border_color.as_deref().and_then(parse_color) {
        el = el.border_color(color);
    }
    // `font-family`: the face this element's text is set in.
    if let Some(ref family) = styles.font_family {
        let family = family.trim().trim_matches(|c| c == '"' || c == '\'');
        if !family.is_empty() {
            el = el.font_family(gpui::SharedString::from(family.to_string()));
        }
    }
    // `font-size`, a pixel value: the text size of this element's text.
    if let Some(ref size) = styles.text_size {
        if let Some(px_val) = parse_px(size) {
            el = el.text_size(px(px_val));
        }
    }
    // `line-height`: a pixel value is the line box's height; a bare number
    // is, as in CSS, a multiple of the font size.
    if let Some(lh) = styles.line_height.as_deref().and_then(parse_line_height) {
        el = el.line_height(lh);
    }
    if let Some(ref weight) = styles.font_weight {
        el = el.font_weight(parse_font_weight(weight));
    }
    // `text-decoration: underline`: a menu's access key, a link.
    if styles
        .text_decoration
        .as_deref()
        .is_some_and(|d| d.split_whitespace().any(|w| w == "underline"))
    {
        el = el.underline();
    }

    // `position: absolute` with its insets: an element drawn over its
    // siblings at a fixed place — a drop zone's translucent quad, a drag's
    // ghost label — without taking a share of the flex layout.
    if styles.position.as_deref().map(str::trim) == Some("absolute") {
        el = el.absolute();
        if let Some(v) = styles.top.as_deref().and_then(parse_px) {
            el = el.top(px(v));
        }
        if let Some(v) = styles.left.as_deref().and_then(parse_px) {
            el = el.left(px(v));
        }
        if let Some(v) = styles.right.as_deref().and_then(parse_px) {
            el = el.right(px(v));
        }
        if let Some(v) = styles.bottom.as_deref().and_then(parse_px) {
            el = el.bottom(px(v));
        }
    }

    el
}

/// A CSS `font-weight` as GPUI's: the keywords and the hundreds.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
pub fn parse_font_weight(s: &str) -> FontWeight {
    match s.trim() {
        "bold" | "bolder" => FontWeight::BOLD,
        "normal" => FontWeight::NORMAL,
        "lighter" => FontWeight::LIGHT,
        other => other
            .parse::<f32>()
            .map(FontWeight)
            .unwrap_or(FontWeight::NORMAL),
    }
}

/// Parse a pixel value from a CSS-like string.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
pub fn parse_px(s: &str) -> Option<f32> {
    let s = s.trim();
    let num_str = s.strip_suffix("px").unwrap_or(s);
    num_str.parse::<f32>().ok()
}

/// Parse a CSS `line-height`: `22px` is absolute, a bare `1.4` is relative
/// to the font size. `normal` and anything unparsable leave GPUI's default.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
pub fn parse_line_height(s: &str) -> Option<gpui::DefiniteLength> {
    let s = s.trim();
    if let Some(n) = s.strip_suffix("px") {
        let v = n.trim().parse::<f32>().ok().filter(|v| *v > 0.0)?;
        return Some(px(v).into());
    }
    let v = s.parse::<f32>().ok().filter(|v| *v > 0.0)?;
    Some(gpui::relative(v))
}

/// Parse a color from a CSS-like string.
/// Returns an Hsla color for use with GPUI's `.bg()` and `.text_color()`.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
pub fn parse_color(s: &str) -> Option<Hsla> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        // `#rrggbbaa`: a translucent fill (a drop zone's quad).
        if hex.len() == 8 {
            let v = u32::from_str_radix(hex, 16).ok()?;
            let rgba_color: Rgba = rgba(v);
            return Some(rgba_color.into());
        }
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let rgba_color: Rgba = rgba(
                ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xff,
            );
            return Some(rgba_color.into());
        }
    }
    // Try named colors (rgb() returns Rgba, convert to Hsla)
    let rgba_color: Option<Rgba> = match s {
        "red" => Some(rgb(0xff0000)),
        "green" => Some(rgb(0x00ff00)),
        "blue" => Some(rgb(0x0000ff)),
        "white" => Some(rgb(0xffffff)),
        "black" => Some(rgb(0x000000)),
        "gray" | "grey" => Some(rgb(0x808080)),
        _ => None,
    };
    rgba_color.map(|c| c.into())
}

// ---------------------------------------------------------------------------
// Application launch
// ---------------------------------------------------------------------------

/// Launch a GPUI application window that renders the shadow tree.
///
/// This function:
/// 1. Creates an `Application` and opens a window
/// 2. Creates a `NimRootView` that reads the shadow tree and produces GPUI elements
/// 3. Starts the GPUI event loop (blocking)
///
/// The shadow tree should already be populated before calling this function
/// (typically via the `root_builder` callback in `gpui_launch`).
///
/// When a `window_id` is provided (non-zero), the window state machine is
/// updated: the window transitions to Visible before the event loop starts
/// and to Closed after the event loop returns.
///
/// # Arguments
/// * `title` - Window title
/// * `width` - Initial window width in pixels
/// * `height` - Initial window height in pixels
/// * `window_id` - The window registry ID (from `gpui_create_window`), or 0
///
/// # Shutdown
///
/// The event loop is stopped through `window::QUIT_REQUESTED` /
/// `window::AUTO_QUIT_MS`, which a task spawned inside the loop polls and
/// turns into `stop_platform_loop` — `cx.quit()` off macOS, and
/// `mac_event_loop::stop_event_loop()` on it, because `App::quit()` there
/// is `-[NSApplication terminate:]` and this function would never return.
/// See the "Shutdown" section of `window.rs` for why the shim needs its
/// own flags rather than a handle to GPUI's `App`.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
pub fn launch_gpui_app(title: &str, width: f64, height: f64, window_id: u32) {
    // Record the active window so components can reference it.
    ACTIVE_WINDOW_ID.store(window_id, std::sync::atomic::Ordering::Release);

    // Transition window to Visible before entering the event loop.
    if window_id != 0 {
        window::show_window(window_id);
    }

    let title_owned = title.to_string();
    let options = window::take_launch_options();
    let requested = window::decorations_override(
        std::env::var("ISONIM_GPUI_DECORATIONS").ok().as_deref(),
    )
    .unwrap_or_else(|value| {
        eprintln!(
            "gpui-nim-shim: ignoring ISONIM_GPUI_DECORATIONS={value:?} \
             (expected \"client\" or \"server\")"
        );
        None
    });
    window::set_requested_decorations(requested);
    let decorations = match requested {
        Some(window::DECORATIONS_CLIENT) => Some(WindowDecorations::Client),
        Some(_) => Some(WindowDecorations::Server),
        None => None,
    };
    let w = width as f32;
    let h = height as f32;

    // Read the deadline ONCE, here, and drop any quit request that was
    // latched before this loop existed. A stale request would otherwise
    // terminate this window before it drew a frame — and a windowed test
    // whose window closes before it paints is exactly the shape of pass
    // that a pixel assertion is supposed to be immune to.
    let auto_quit_ms = window::auto_quit_ms();
    window::clear_quit_request();

    // macOS: `-[NSApplication run]` sends `applicationDidFinishLaunching:`
    // once per PROCESS, and that notification is the only thing that calls
    // the launch closure `MacPlatform::run` parks in `state.finish_launching`.
    // The second `gpui_launch` in a process would therefore open no window
    // and spawn no shutdown poller — a hang, not a missed frame. Queued
    // before the loop is entered so it runs inside it, after the delegate
    // is installed. See `mac_event_loop.rs`.
    #[cfg(target_os = "macos")]
    if crate::mac_event_loop::begin_launch() {
        crate::mac_event_loop::kick_relaunch();
    }

    // RS-M14 Phase 2: pinned `gpui` requires an explicit platform implementation
    // (the old crates.io `Application::new()` constructor is gone). Use
    // `current_platform(false)` to get the windowed (non-headless) platform impl
    // appropriate for the current OS.
    // Bind the platform so the font-smoothing policy can be applied through
    // the SAME text system this app will render with, before any glyph is
    // rasterised. `current_platform` is called once, not twice: the policy's
    // memo is process-global, but creating a second platform here would also
    // create a second AppKit app object. See
    // `window::apply_chromium_font_smoothing_policy`.
    let platform = current_platform(false);
    #[cfg(target_os = "macos")]
    let _font_smoothing =
        crate::window::apply_and_report_font_smoothing_policy(platform.text_system().as_ref());

    // macOS: let the window keep the size it is about to be asked for.
    // `GPUIWindow` inherits `NSWindow`'s `constrainFrameRect:toScreen:`,
    // which fits a window to the screen's visible frame — so a viewport
    // larger than this display came back silently shrunk, where Chromium
    // (and therefore the Electron front-end) returns the proposed frame
    // unchanged and gets the size it asked for. See
    // `mac_window_frame.rs` for the measurement and the scope.
    //
    // BEFORE `run`, and that is load-bearing: the first constraining
    // happens inside `initWithContentRect:…`, so an override installed
    // once a window exists is already too late for that window's frame.
    #[cfg(target_os = "macos")]
    let window_frame_override = crate::mac_window_frame::install_unconstrained_window_frame();
    #[cfg(target_os = "macos")]
    if !window_frame_override.is_installed() {
        // Not fatal — a clamped window still runs — but it is the one
        // state in which a `wide` capture silently measures a different
        // window than the one the scenario named, so it is never left to
        // be inferred from the pixels.
        eprintln!(
            "gpui-nim-shim: window frame override NOT installed ({window_frame_override:?}); \
             a window larger than the display will be clamped to its visible frame"
        );
    }

    Application::with_platform(platform).run(move |cx: &mut App| {
        // The shim owns the quit policy; see `spawn_shutdown_poller`.
        // `QuitMode::Default` is `LastWindowClosed` off macOS, which
        // quits INSIDE the update that removes the last window and so
        // leaves the window's teardown unflushed. The poller reproduces
        // the same user-visible rule (last window gone => app quits) with
        // a drain in between.
        cx.set_quit_mode(QuitMode::Explicit);
        // Text antialiasing as Chromium picks it on this desktop
        // (`window::text_antialiasing`).
        match crate::window::text_antialiasing(
            std::env::var("ISONIM_GPUI_TEXT_RENDERING").ok().as_deref(),
            crate::window::fontconfig_rgba().as_deref(),
        ) {
            Some(crate::window::TextAntialiasing::Grayscale) => {
                cx.set_text_rendering_mode(gpui::TextRenderingMode::Grayscale)
            }
            Some(crate::window::TextAntialiasing::Subpixel) => {
                cx.set_text_rendering_mode(gpui::TextRenderingMode::Subpixel)
            }
            None => {}
        }
        register_fonts(cx);

        // GWIN: the title, identity, minimum size and initial state the
        // application asked for. A maximized window keeps the launch size as
        // its restore size.
        // GWIN: the centred origin is SNAPPED TO WHOLE POINTS, because a
        // fractional one costs the window the size it asked for.
        //
        // `Bounds::centered` halves the leftover display extent, so whenever
        // the display extent and the launch size differ by an ODD number of
        // points the origin lands on a half point. Measured on this host
        // (aarch64-darwin, macOS 26.5.2, 1728x1117-point display, 1440x900
        // launch): x is (1728-1440)/2 = 144 exactly, but y is (1117-900)/2 =
        // **108.5**. A window opened at a fractional origin does not get the
        // content area it asked for — AppKit snaps the origin down to a whole
        // point and GROWS the content view by one point to cover the
        // remainder. Driven directly against AppKit with gpui's own style
        // mask (titled|closable|resizable|miniaturizable), a 900-point
        // content request produced a 901-point content view at y =
        // 108.25/108.5/108.75 and exactly 900 at y = 108 or 109. The width
        // was never wrong here for the one reason that its centred origin
        // happened to be integral.
        //
        // One surplus point is not cosmetic: the top bar is top-anchored and
        // the status bar bottom-anchored, so the surplus lands in the pane
        // area between them and every row below the top bar is drawn one
        // point lower than the layout says.
        //
        // This is NOT macOS-specific and is deliberately not behind a
        // `cfg(target_os)`. A window origin is a whole number of pixels on
        // X11 and Wayland as well, so a fractional origin is an artefact of
        // the centring arithmetic on every platform rather than a position
        // any of them can honour; rounding it is correct everywhere and
        // changes nothing on a display whose arithmetic already came out
        // even.
        let mut bounds = Bounds::centered(None, size(px(w), px(h)), cx);
        bounds.origin.x = bounds.origin.x.round();
        bounds.origin.y = bounds.origin.y.round();
        let maximized = options.maximized;
        cx.open_window(
            WindowOptions {
                window_bounds: Some(if options.maximized {
                    WindowBounds::Maximized(bounds)
                } else {
                    WindowBounds::Windowed(bounds)
                }),
                titlebar: Some(TitlebarOptions {
                    title: Some(title_owned.into()),
                    ..Default::default()
                }),
                app_id: options.app_id,
                window_min_size: options
                    .min_size
                    .map(|(mw, mh)| size(px(mw as f32), px(mh as f32))),
                window_decorations: decorations,
                ..Default::default()
            },
            move |window, cx| {
                // GWIN: a window GPUI expects to decorate itself (on Wayland
                // that is its state until the compositor answers) is created
                // larger by the frame's band, so that its content is the
                // launch size from the first commit on. See
                // `NimRootView::launch_inflated`. A maximized launch keeps the
                // size it was given: the compositor sizes it.
                let inflated = if !maximized
                    && matches!(window.window_decorations(), Decorations::Client { .. })
                {
                    let band = crate::frame::FRAME_INSET;
                    window.set_client_inset(px(band));
                    window.resize(size(px(w + 2.0 * band), px(h + 2.0 * band)));
                    Some((w, h))
                } else {
                    None
                };
                cx.new(|_| {
                    let mut view = NimRootView::new();
                    view.launch_inflated = inflated;
                    view
                })
            },
        )
        .expect("Failed to open GPUI window");

        spawn_shutdown_poller(cx, auto_quit_ms);
        spawn_ticker(cx);
    });

    // macOS: `MacPlatform::run` nulls the platform pointer in the delegate
    // it created but leaves that delegate registered as a notification
    // observer, and it creates a new one per launch. Unregister it here so
    // a later keyboard-layout or thermal notification cannot reach a
    // delegate whose ivar is null. See `mac_event_loop.rs`.
    #[cfg(target_os = "macos")]
    crate::mac_event_loop::release_delegate_observers();

    // The event loop has returned -- the user closed the window, or a
    // quit was requested via `gpui_quit` / the auto-quit deadline.
    // A window already closed through `close_window` (its close button, a
    // compositor close, `gpui_window_request_close`) has had its close
    // callback; it is not asked again.
    if window_id != 0 && window::window_state(window_id) != Some(window::WindowState::Closed) {
        window::close_window(window_id);
    }

    // Disarm, so the deadline this launch was given is not inherited by
    // the next one.
    window::set_auto_quit_ms(0);
    window::clear_quit_request();
    window::set_tick(0, None);

    ACTIVE_WINDOW_ID.store(0, std::sync::atomic::Ordering::Release);
}

/// How long the loop keeps running after the last window has been
/// removed, before `stop_platform_loop` stops it.
///
/// This is not politeness, it is the difference between a window that is
/// gone and a window that merely thinks it is. `Drop for WaylandWindow`
/// (gpui_linux/src/linux/wayland/window.rs) sends `wl_surface.destroy`
/// and then spawns `client.drop_window(..)` on the FOREGROUND EXECUTOR —
/// so both the protocol flush and that task need the event loop to keep
/// turning for a moment. Stop the loop in the same update that removed
/// the window and the compositor never hears about it.
///
/// WHAT IS MEASURED, AND WHAT THIS NUMBER ACTUALLY IS. The two were
/// conflated in an earlier draft of this comment, so they are separated
/// here. `tests/test_gui.nim` makes six `gpui_launch` calls in one
/// process, and the experiments below are over that binary.
///
///   * REMOVING THE WINDOWS IS LOAD-BEARING, and it is what the 11%
///     measurement belongs to. With the poller replaced by a bare
///     `cx.quit()` that removes nothing, all six surfaces stay alive:
///     sway tiles the sixth window into a 320x1080 column and the pixel
///     case never sees a paintable frame. Re-measured 2026-09-17, twice,
///     both runs red at exactly 698,940 of 6,220,817 non-NUL bytes
///     (11.2%), with the 7,740-byte teardown transient before it.
///
///   * THE LENGTH OF THIS WAIT IS NOT LOAD-BEARING, and calling it
///     "measured" would be overclaiming. With `SHUTDOWN_DRAIN` set to
///     ZERO the pixel case still passes, twice, with byte-identical
///     counts — because the poller `continue`s and re-enters the
///     `timer(SHUTDOWN_POLL).await` before it tests the deadline, so a
///     zero drain is still one 16 ms tick and, crucially, still puts
///     `cx.quit()` in a STRICTLY LATER app update than the removal.
///
/// That separation is the real condition: `Drop for WaylandWindow`
/// spawns `client.drop_window(..)` on the foreground executor, and a
/// task cannot run in the update that queued it. The condition is
/// already structural, not timed — this constant only buys margin on top
/// of it.
///
/// 150 ms is therefore a deliberately generous margin, not a threshold
/// anyone measured a failure below. It is kept because the platforms
/// this has NOT been measured on are the ones that matter here — the
/// ephemeral Linux runners, and arm64, where no figure in this file was
/// taken — and nine spare poll ticks cost one sixth of a second per
/// launch. If that ever becomes expensive, the honest replacement is the
/// condition itself (quit on the first tick after the removal), not a
/// smaller magic number.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
const SHUTDOWN_DRAIN: std::time::Duration = std::time::Duration::from_millis(150);

/// Polling interval for the shutdown task.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
const SHUTDOWN_POLL: std::time::Duration = std::time::Duration::from_millis(16);

/// End the platform event loop so `Application::run` — and with it
/// `launch_gpui_app`, and with IT the `gpui_launch` FFI call — returns.
///
/// **This is not `App::quit()` everywhere, and that is the whole point.**
/// `App::quit()` is `self.platform.quit()`, and the two platforms disagree
/// about what that means: Linux stops the calloop `LoopSignal` and `run`
/// returns, macOS calls `-[NSApplication terminate:]` and the process ends
/// without unwinding. Every line after `Application::run(...)` below, and
/// every line after `gpui_launch` in every consumer, is dead code under
/// the second reading. See `mac_event_loop.rs` for the measurement, the
/// three candidate routes and why this one was taken.
///
/// Must run inside an app update on the main thread; both callers are the
/// shutdown poller, which runs on the foreground executor for exactly that
/// reason.
#[cfg(all(
    any(feature = "gpui-backend", feature = "gpui-headless"),
    not(target_os = "macos")
))]
fn stop_platform_loop(cx: &mut App) {
    cx.quit();
}

#[cfg(all(
    any(feature = "gpui-backend", feature = "gpui-headless"),
    target_os = "macos"
))]
fn stop_platform_loop(_cx: &mut App) {
    // A refusal here means `stop:` was armed and no event will wake the
    // loop to read it, i.e. the process would hang rather than return.
    // That is worth a line on stderr: the alternative is a silent hang
    // whose only symptom is a lane timing out at its cap.
    if !crate::mac_event_loop::stop_event_loop() {
        eprintln!(
            "gpui-nim-shim: could not stop the AppKit event loop \
             (not on the main thread, or NSEvent refused the wake event)"
        );
    }
}

/// Spawn the task that owns this app's shutdown.
///
/// It runs on the foreground executor (the same place `NimRootView`'s
/// repaint poller runs) because `App::quit` may only be reached from
/// inside an app update, and `Rc<dyn Platform>` cannot be handed to
/// another thread. That is also why the shim needs
/// `window::QUIT_REQUESTED` at all: `gpui_quit` can be called from
/// anywhere, and this is the one place allowed to act on it.
///
/// Three things end the loop, and all three take the same route out —
/// remove the windows, drain, quit:
///
///   1. `gpui_quit()` from any thread.
///   2. The `gpui_quit_after_ms` deadline, if one was armed.
///   3. The last window disappearing, i.e. the user closed it. This
///      reproduces `QuitMode::LastWindowClosed`, which `launch_gpui_app`
///      turns off precisely so that the drain can happen in between.
///
/// `auto_quit_ms == 0` means "no deadline"; only (1) and (3) apply.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
fn spawn_shutdown_poller(cx: &mut App, auto_quit_ms: u32) {
    let deadline = if auto_quit_ms == 0 {
        None
    } else {
        Some(std::time::Duration::from_millis(auto_quit_ms as u64))
    };

    cx.spawn(async move |cx: &mut AsyncApp| {
        let started = std::time::Instant::now();
        let mut draining_since: Option<std::time::Instant> = None;

        loop {
            cx.background_executor().timer(SHUTDOWN_POLL).await;

            if let Some(since) = draining_since {
                if since.elapsed() >= SHUTDOWN_DRAIN {
                    cx.update(stop_platform_loop);
                    break;
                }
                continue;
            }

            let asked = window::take_quit_request();
            let expired = deadline.is_some_and(|d| started.elapsed() >= d);
            let all_windows_gone = cx.update(|cx: &mut App| cx.windows().is_empty());

            if asked || expired || all_windows_gone {
                cx.update(|cx: &mut App| {
                    for handle in cx.windows() {
                        let _ =
                            handle.update(cx, |_root, win: &mut Window, _cx| win.remove_window());
                    }
                });
                draining_since = Some(std::time::Instant::now());
            }
        }
    })
    .detach();
}

/// How often the ticker looks for an armed tick while none is armed — the
/// same slice as a re-arm's, so arming from idle is as prompt as re-arming.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
const TICK_IDLE_POLL: std::time::Duration = TICK_REARM_POLL;

/// The longest the ticker sleeps before looking whether the host re-armed
/// its tick: the latency a re-arm can add to a short deadline.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
const TICK_REARM_POLL: std::time::Duration = std::time::Duration::from_millis(50);

/// Run the host's tick (`gpui_set_tick`) inside the loop: wait its interval,
/// call it on the main thread (`cx.update`, where GPUI requires app state to
/// be touched), request a repaint. The wait is taken in slices of at most
/// `TICK_REARM_POLL`, and a re-arm or disarm (`window::tick_generation`)
/// restarts it — so a host that armed a long refresh can arm a short
/// deadline and have it honoured, rather than waiting out the long one. The
/// task ends with the loop.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
fn spawn_ticker(cx: &mut App) {
    cx.spawn(async move |cx: &mut AsyncApp| loop {
        let generation = window::tick_generation();
        let Some((interval_ms, _)) = window::tick() else {
            cx.background_executor().timer(TICK_IDLE_POLL).await;
            continue;
        };
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(interval_ms as u64);
        let mut rearmed = false;
        loop {
            let now = std::time::Instant::now();
            if now >= deadline {
                break;
            }
            cx.background_executor()
                .timer((deadline - now).min(TICK_REARM_POLL))
                .await;
            if window::tick_generation() != generation {
                rearmed = true;
                break;
            }
        }
        if rearmed {
            continue;
        }
        if let Some((_, callback)) = window::tick() {
            cx.update(|_cx: &mut App| callback());
            window::request_repaint();
        }
    })
    .detach();
}

#[cfg(test)]
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
mod tests {
    // Do NOT use `use super::*` here because it would bring gpui traits
    // (re-exported via use statements) that include a `test` proc macro
    // which shadows `#[test]` and causes infinite macro recursion.
    use super::{active_window_id, launch_gpui_app, parse_line_height};

    #[test]
    fn test_launch_gpui_app_exists() {
        // Verify the function exists and has the right signature.
        let _f: fn(&str, f64, f64, u32) = launch_gpui_app;
    }

    #[test]
    fn test_active_window_id_default() {
        assert_eq!(active_window_id(), 0);
    }

    #[test]
    fn test_line_height_is_absolute_in_px_and_relative_bare() {
        assert_eq!(
            parse_line_height("22px"),
            Some(gpui::DefiniteLength::Absolute(gpui::AbsoluteLength::Pixels(
                gpui::px(22.0)
            )))
        );
        assert_eq!(
            parse_line_height(" 1.5 "),
            Some(gpui::DefiniteLength::Fraction(1.5))
        );
        assert_eq!(parse_line_height("normal"), None);
        assert_eq!(parse_line_height("0px"), None);
    }
}
