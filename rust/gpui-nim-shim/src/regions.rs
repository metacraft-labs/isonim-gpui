//! Window regions: parts of the application's own tree that act as its title
//! bar — the IsoNim counterpart of Electron's `-webkit-app-region` and of
//! GPUI's `WindowControlArea`.
//!
//! An element declares one with the attribute `data-window-region`:
//!
//! | value      | a press on it                                               |
//! |------------|-------------------------------------------------------------|
//! | `drag`     | left: moves the window on the first motion, or toggles      |
//! |            | maximize on a double-click; right: the window menu          |
//! | `no-drag`  | is the application's, never the window's                    |
//! | `minimize` | a click minimizes the window                                |
//! | `maximize` | a click toggles maximize                                    |
//! | `close`    | a click closes the window through its normal close path     |
//!
//! **The topmost element under the pointer decides.** A press that an
//! element in front of a drag region takes — a `no-drag` element, a window
//! button, or any element with a pointer listener — does not move the
//! window, whether that element is a child of the drag region or a sibling
//! drawn over it. GPUI delivers a press to every element under the pointer,
//! front to back; the ones in front CLAIM it (`claim`) and the drag region,
//! further back, finds it claimed. The root clears the claim at the start of
//! every press (`begin_press`, in the capture phase).
//!
//! Drag regions act only while the application draws the decorations
//! (`window::client_decorated`), so an application can declare them
//! unconditionally; window buttons act in either mode.
//!
//! This module holds the parsing and the per-press state, with no GPUI types;
//! `gpui_app` wires them to elements.

use std::cell::Cell;

/// The attribute an element declares its region with.
pub const REGION_ATTRIBUTE: &str = "data-window-region";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Drag,
    NoDrag,
    Minimize,
    Maximize,
    Close,
}

/// The region an attribute value names; `None` for anything else, which is
/// then an ordinary element.
pub fn parse_region(value: &str) -> Option<Region> {
    match value.trim() {
        "drag" => Some(Region::Drag),
        "no-drag" => Some(Region::NoDrag),
        "minimize" => Some(Region::Minimize),
        "maximize" => Some(Region::Maximize),
        "close" => Some(Region::Close),
        _ => None,
    }
}

/// The events that make an element interactive, and so implicitly
/// `no-drag`: anything that listens for the pointer.
pub fn is_pointer_listener(has_click_handler: bool, event_names: &[String]) -> bool {
    has_click_handler
        || event_names.iter().any(|n| {
            matches!(
                n.as_str(),
                "click" | "mousedown" | "mouseup" | "mousemove" | "contextmenu" | "auxdown"
            )
        })
}

// GPUI dispatches input on the main thread, so the per-press state is
// thread-local: nothing else can observe it half-updated.
thread_local! {
    static CLAIMED: Cell<bool> = const { Cell::new(false) };
    static MOVE_ARMED: Cell<bool> = const { Cell::new(false) };
}

/// A new press is being dispatched: nothing has claimed it yet.
pub fn begin_press() {
    CLAIMED.with(|c| c.set(false));
}

/// An element in front of any drag region takes this press.
pub fn claim() {
    CLAIMED.with(|c| c.set(true));
}

pub fn claimed() -> bool {
    CLAIMED.with(|c| c.get())
}

/// A left press on a drag region: the window moves on the first motion
/// while the button is held. Kept between frames — the tree is rebuilt on
/// every one, so the press and its motion are handled by different elements.
pub fn arm_move() {
    MOVE_ARMED.with(|c| c.set(true));
}

/// Whether a move is armed, disarming it: one press starts one move.
pub fn take_move() -> bool {
    MOVE_ARMED.with(|c| c.replace(false))
}

pub fn disarm_move() {
    MOVE_ARMED.with(|c| c.set(false));
}

/// Whether any element of the tree under `root` declares a drag region —
/// the application has a title bar of its own, and the renderer draws none.
pub fn tree_declares_drag_region(tree: &crate::tree::Tree, root: crate::tree::NodeId) -> bool {
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let Some(node) = tree.get(id) else { continue };
        if node
            .attributes
            .get(REGION_ATTRIBUTE)
            .and_then(|v| parse_region(v))
            == Some(Region::Drag)
        {
            return true;
        }
        stack.extend(node.children.iter().copied());
    }
    false
}

// ---------------------------------------------------------------------------
// The fallback title bar's buttons
// ---------------------------------------------------------------------------
//
// The same policy as `isonim_gpui/window_environment.resolveWindowEnvironment`
// (which applications use for their OWN title bars), restated here because the
// renderer draws the fallback bar with no application code involved. The two
// are tested over the same cases.

/// The fallback title bar's height in logical pixels.
pub const FALLBACK_BAR_HEIGHT: f32 = 32.0;

/// The buttons for the left and right ends of a title bar: the desktop's
/// configured layout (GNOME `button-layout` syntax, as `window::ChromeState`
/// carries it), or the platform convention when there is none, less the
/// buttons whose operation the platform does not support. Close always is.
pub fn title_bar_buttons(layout: &str, controls: u8, macos: bool) -> (Vec<Region>, Vec<Region>) {
    use crate::window::{CONTROL_MAXIMIZE, CONTROL_MINIMIZE};
    let supported = |r: Region| match r {
        Region::Minimize => controls & CONTROL_MINIMIZE != 0,
        Region::Maximize => controls & CONTROL_MAXIMIZE != 0,
        _ => true,
    };
    let (left, right) = match layout.split_once(':') {
        Some((l, r)) => {
            let mut seen = Vec::new();
            let mut side = |text: &str| -> Vec<Region> {
                text.split(',')
                    .filter_map(|n| match n.trim() {
                        "minimize" => Some(Region::Minimize),
                        "maximize" => Some(Region::Maximize),
                        "close" => Some(Region::Close),
                        _ => None,
                    })
                    .filter(|r| {
                        let first = !seen.contains(r);
                        seen.push(*r);
                        first
                    })
                    .collect()
            };
            let l = side(l);
            (l, side(r))
        }
        None if macos => (
            vec![Region::Close, Region::Minimize, Region::Maximize],
            vec![],
        ),
        None => (
            vec![],
            vec![Region::Minimize, Region::Maximize, Region::Close],
        ),
    };
    (
        left.into_iter().filter(|r| supported(*r)).collect(),
        right.into_iter().filter(|r| supported(*r)).collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_values_parse_and_others_do_not() {
        assert_eq!(parse_region("drag"), Some(Region::Drag));
        assert_eq!(parse_region(" no-drag "), Some(Region::NoDrag));
        assert_eq!(parse_region("minimize"), Some(Region::Minimize));
        assert_eq!(parse_region("maximize"), Some(Region::Maximize));
        assert_eq!(parse_region("close"), Some(Region::Close));
        assert_eq!(parse_region("Drag"), None);
        assert_eq!(parse_region(""), None);
    }

    #[test]
    fn pointer_listeners_are_interactive_and_keys_are_not() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(is_pointer_listener(true, &[]));
        assert!(is_pointer_listener(false, &names(&["mousedown"])));
        assert!(is_pointer_listener(false, &names(&["contextmenu"])));
        assert!(!is_pointer_listener(false, &names(&["keydown", "wheel"])));
    }

    #[test]
    fn a_claim_lasts_until_the_next_press() {
        begin_press();
        assert!(!claimed());
        claim();
        assert!(claimed());
        begin_press();
        assert!(!claimed());
    }

    #[test]
    fn one_press_arms_one_move() {
        disarm_move();
        assert!(!take_move());
        arm_move();
        assert!(take_move());
        assert!(!take_move());
        arm_move();
        disarm_move();
        assert!(!take_move());
    }

    // The same cases as tests/test_window_environment.nim's
    // resolveWindowEnvironment suite.
    const ALL: u8 = 15;
    use Region::{Close, Maximize, Minimize};

    #[test]
    fn title_bar_buttons_follow_the_configured_layout() {
        assert_eq!(
            title_bar_buttons(":minimize,maximize,close", ALL, false),
            (vec![], vec![Minimize, Maximize, Close])
        );
        assert_eq!(
            title_bar_buttons("close,minimize,maximize:", ALL, false),
            (vec![Close, Minimize, Maximize], vec![])
        );
        assert_eq!(
            title_bar_buttons("appmenu:close", ALL, false),
            (vec![], vec![Close])
        );
        assert_eq!(
            title_bar_buttons("icon,close:spacer,close,maximize,maximize", ALL, false),
            (vec![Close], vec![Maximize])
        );
        assert_eq!(title_bar_buttons(":", ALL, false), (vec![], vec![]));
    }

    #[test]
    fn title_bar_buttons_drop_what_the_platform_cannot_do() {
        // sway: maximize and fullscreen only.
        assert_eq!(
            title_bar_buttons(":minimize,maximize,close", 2 | 4, false),
            (vec![], vec![Maximize, Close])
        );
        assert_eq!(
            title_bar_buttons(":minimize,maximize,close", 0, false),
            (vec![], vec![Close])
        );
    }

    #[test]
    fn title_bar_buttons_default_to_the_platform_convention() {
        assert_eq!(
            title_bar_buttons("", ALL, false),
            (vec![], vec![Minimize, Maximize, Close])
        );
        assert_eq!(
            title_bar_buttons("close,minimize", ALL, false),
            (vec![], vec![Minimize, Maximize, Close])
        );
        assert_eq!(
            title_bar_buttons("", ALL, true),
            (vec![Close, Minimize, Maximize], vec![])
        );
    }
}
