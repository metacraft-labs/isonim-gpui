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
    AsyncApp, Bounds, Context, Div, FontWeight, Hsla, InteractiveElement, IntoElement,
    MouseButton, ParentElement, QuitMode, Render, Rgba, Styled, WeakEntity, Window, WindowBounds,
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
}

#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
impl NimRootView {
    pub fn new() -> Self {
        NimRootView {
            poll_started: false,
            focus_handle: None,
        }
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

        let tree = crate::lock_tree();
        let root_id = *crate::ROOT_NODE_ID.lock().unwrap_or_else(|p| p.into_inner());

        if root_id.is_null() {
            return with_keyboard(div().size_full().child("No shadow tree root"), &handle)
                .into_any_element();
        }

        let started = std::time::Instant::now();
        match crate::render_sync::build_render_plan(&tree, root_id) {
            Some(plan) => {
                drop(tree); // release lock before building GPUI elements
                let element = render_root_to_gpui(&plan, &handle);
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

    // Wire click events
    if plan.has_click_handler {
        let node_id = crate::tree::NodeId(plan.node_id);
        el = el.on_mouse_up(MouseButton::Left, move |_event, _window, _cx| {
            dispatch_shadow_event(node_id, "click");
        });
    }

    el = wire_pointer_listeners(el, plan);
    el
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
/// Only the left button presses and releases: a drag is a left-button
/// gesture, and a listener for every button would hand a right-click to a
/// consumer that asked for a drag. A move is reported whether or not a button
/// is down; the consumer knows whether a gesture is in flight.
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
    if let Some(ref weight) = styles.font_weight {
        el = el.font_weight(parse_font_weight(weight));
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

    let _title_static: &'static str = Box::leak(title.to_string().into_boxed_str());
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
    Application::with_platform(current_platform(false)).run(move |cx: &mut App| {
        // The shim owns the quit policy; see `spawn_shutdown_poller`.
        // `QuitMode::Default` is `LastWindowClosed` off macOS, which
        // quits INSIDE the update that removes the last window and so
        // leaves the window's teardown unflushed. The poller reproduces
        // the same user-visible rule (last window gone => app quits) with
        // a drain in between.
        cx.set_quit_mode(QuitMode::Explicit);

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(
                    Bounds::centered(None, size(px(w), px(h)), cx),
                )),
                ..Default::default()
            },
            |_, cx| cx.new(|_| NimRootView::new()),
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
    if window_id != 0 {
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
    use super::{active_window_id, launch_gpui_app};

    #[test]
    fn test_launch_gpui_app_exists() {
        // Verify the function exists and has the right signature.
        let _f: fn(&str, f64, f64, u32) = launch_gpui_app;
    }

    #[test]
    fn test_active_window_id_default() {
        assert_eq!(active_window_id(), 0);
    }
}
