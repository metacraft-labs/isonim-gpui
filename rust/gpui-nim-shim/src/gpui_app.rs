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
    div, px, rgb, rgba, size, AnyElement, App, AppContext as _, Application, AsyncApp, Bounds,
    Context, Div, Hsla, InteractiveElement, IntoElement, MouseButton, ParentElement, QuitMode,
    Render, Rgba, Styled, WeakEntity, Window, WindowBounds, WindowOptions,
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
}

#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
impl NimRootView {
    pub fn new() -> Self {
        NimRootView {
            poll_started: false,
        }
    }
}

#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
impl Render for NimRootView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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

        let tree = crate::lock_tree();
        let root_id = *crate::ROOT_NODE_ID.lock().unwrap_or_else(|p| p.into_inner());

        if root_id.is_null() {
            return div().size_full().child("No shadow tree root").into_any_element();
        }

        match crate::render_sync::build_render_plan(&tree, root_id) {
            Some(plan) => {
                drop(tree); // release lock before building GPUI elements
                render_plan_to_gpui(&plan).into_any_element()
            }
            None => div().size_full().child("Empty shadow tree").into_any_element(),
        }
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
                el = el.on_mouse_up(
                    MouseButton::Left,
                    move |_event, _window, _cx| {
                        dispatch_shadow_event(node_id, "click");
                    },
                );
            }

            el.into_any_element()
        }
    }
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

    el
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
/// turns into `cx.quit()`. See the "Shutdown" section of `window.rs` for
/// why the shim needs its own flags rather than a handle to GPUI's `App`.
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
    });

    // The event loop has returned -- the user closed the window, or a
    // quit was requested via `gpui_quit` / the auto-quit deadline.
    if window_id != 0 {
        window::close_window(window_id);
    }

    // Disarm, so the deadline this launch was given is not inherited by
    // the next one.
    window::set_auto_quit_ms(0);
    window::clear_quit_request();

    ACTIVE_WINDOW_ID.store(0, std::sync::atomic::Ordering::Release);
}

/// How long the loop keeps running after the last window has been
/// removed, before `cx.quit()` stops it.
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
                    cx.update(|cx: &mut App| cx.quit());
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
