//! GPUI application lifecycle: launching the real GPUI window with the
//! shadow tree renderer as the root view.
//!
//! This module is only compiled when the `gpui-backend` feature is enabled.
//! It provides `launch_gpui_app()` which creates a GPUI window, registers
//! the shadow tree renderer view, and starts the event loop.

#[cfg(feature = "gpui-backend")]
use crate::render_sync::gpui_render::dispatch_shadow_event;
#[cfg(feature = "gpui-backend")]
use crate::window;

/// The ID of the window that is currently being displayed by GPUI.
/// Set before launching the event loop so that the root view and
/// event handlers can reference the correct window in the registry.
#[cfg(feature = "gpui-backend")]
static ACTIVE_WINDOW_ID: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);

/// Get the active GPUI window ID (0 if none).
#[cfg(feature = "gpui-backend")]
#[allow(dead_code)]
pub fn active_window_id() -> u32 {
    ACTIVE_WINDOW_ID.load(std::sync::atomic::Ordering::Acquire)
}

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
#[cfg(feature = "gpui-backend")]
pub fn launch_gpui_app(title: &str, width: f64, height: f64, window_id: u32) {
    use gpui::*;

    // Record the active window so components can reference it.
    ACTIVE_WINDOW_ID.store(window_id, std::sync::atomic::Ordering::Release);

    // Transition window to Visible before entering the event loop.
    if window_id != 0 {
        window::show_window(window_id);
    }

    // NimRootView: reads the shadow tree and produces GPUI elements.
    struct NimRootView;

    impl Render for NimRootView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            // Check and clear the repaint flag (so we know the frame is current).
            let _ = window::take_repaint_request();

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

    /// Recursively convert a RenderNode to GPUI AnyElement.
    fn render_plan_to_gpui(plan: &crate::render_sync::RenderNode) -> AnyElement {
        use crate::tree::GpuiElementKind;

        match plan.kind {
            GpuiElementKind::TextNode => {
                let text = plan.text.clone().unwrap_or_default();
                text.into_any_element()
            }
            GpuiElementKind::Img => {
                // TODO: load actual image from src attribute
                let el = div();
                apply_styles_to_div(el, &plan.styles).into_any_element()
            }
            GpuiElementKind::Svg => {
                // TODO: load actual SVG from attribute
                let el = div();
                apply_styles_to_div(el, &plan.styles).into_any_element()
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
    fn apply_styles_to_div(
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
    fn parse_px(s: &str) -> Option<f32> {
        let s = s.trim();
        let num_str = s.strip_suffix("px").unwrap_or(s);
        num_str.parse::<f32>().ok()
    }

    /// Parse a color from a CSS-like string.
    /// Returns an Hsla color for use with GPUI's `.bg()` and `.text_color()`.
    fn parse_color(s: &str) -> Option<Hsla> {
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

    // -- Launch the application --

    let _title_static: &'static str = Box::leak(title.to_string().into_boxed_str());
    let w = width as f32;
    let h = height as f32;

    Application::new().run(move |cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(
                    Bounds::centered(None, size(px(w), px(h)), cx),
                )),
                ..Default::default()
            },
            |_, cx| cx.new(|_| NimRootView),
        )
        .expect("Failed to open GPUI window");
    });

    // The event loop has returned -- the user closed the window.
    if window_id != 0 {
        window::close_window(window_id);
    }

    ACTIVE_WINDOW_ID.store(0, std::sync::atomic::Ordering::Release);
}

#[cfg(test)]
#[cfg(feature = "gpui-backend")]
mod tests {
    use super::*;

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
