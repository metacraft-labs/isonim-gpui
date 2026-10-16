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
}
