//! PLAT-38 — element focus, and event delivery that carries a payload.
//!
//! # What this module exists to close
//!
//! Three defects were measured against this shim by `codetracer`'s PLAT-21
//! and filed, with the count of vocabulary entries each affects:
//!
//! | id | entries | what a binding could not do |
//! |---|---|---|
//! | `PLAT21-VG1` | 12 | **A key could not be delivered.** `addEventListener` took a `proc()`, `gpui_dispatch_event` carried no payload, and there was no keyboard event at all. The one binding that needed a key encoded it in the event NAME — `vockey:Down` — because the name was the only channel with room in it. |
//! | `PLAT21-VG2` | 12 | `setAttribute(el, "disabled", v)` was rewritten to `enabled` and its value constant-folded to `"false"`, so saying "enabled" recorded "disabled" and `getAttribute(el, "disabled")` answered `""`. (That half is Nim-side, in `renderer.nim`.) |
//! | `PLAT21-VG3` | 1 (`Modal`) | **No element focus.** Focus was per WINDOW. Exclusivity is what that entry IS. |
//!
//! VG1 and VG3 are this file. The block was at the **C ABI**, not at the view
//! layer: no handler could be attached that would have helped, because there
//! was nothing for a handler to be handed.
//!
//! # The three things a reader should check before trusting anything here
//!
//! **1. The payload is DATA, not a rendered string.** A key arrives as a base
//! name (`"Up"`, `"F10"`, `"a"`, `"Space"`, `"é"`) plus a modifier BITMASK,
//! never as `"Shift+F10"`. `Verification-Harness-Traps.md` §25 is the reason:
//! a helper that silently drops a modifier it cannot spell hands you a test
//! about a different key, and that has been paid for once already in this
//! workspace (`TermAssert.sendKey` consumed `shift+` and forgot it). A
//! modifier cannot fall out of a bitmask without the number changing, and the
//! number is what the gate asserts.
//!
//! **2. Focus is a PER-NODE FLAG, and that is deliberately more expensive
//! than a single remembered id.** See `tree::Node::focused`. A single id makes
//! exclusivity true by construction and PLAT-38's focus partition law
//! unfalsifiable — §36's exact shape, a published killer that cannot kill its
//! own law. `focus_only` establishes exclusivity by clearing every flag;
//! `focused_ids` observes it by counting them; the published killer ("let two
//! elements hold it") is performable and lands.
//!
//! **3. The element store records the delivery BEFORE any callback runs.**
//! `last_event` and `delivery_count` on the node are written on the Rust side
//! of the FFI boundary. That is what makes them an oracle a Nim-side binding
//! cannot forge: a binding that applied a key on its own side, or an adapter
//! that emulated delivery by walking the shadow tree in Nim (which is what
//! `isonim-render-serve`'s `gpui_input_adapter.nim` does today), leaves both
//! at their initial values. PLAT-38's gate reads them after a key that came
//! in through the compositor's own `wl_seat`.
//!
//! # One delivery routine, not two
//!
//! `gpui_dispatch_event` (the C ABI) and
//! `render_sync::gpui_render::dispatch_shadow_event` (the real GPUI event
//! handlers) were two byte-identical copies of one routine. The gate reads
//! the store after a compositor key — the second copy — while most cases
//! drive the first. Two copies of one predicate is §30, and here it would
//! have meant grading a different function from the one under test. Both call
//! `deliver`.

use std::os::raw::c_char;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::tree::{DeliveredEvent, NodeId};
use crate::{cstr_to_str, handle_to_node_id, lock_tree, node_id_to_handle, GpuiElement};

// ---------------------------------------------------------------------------
// The payload
// ---------------------------------------------------------------------------

/// The event carries nothing beyond its name (a click, a legacy dispatch).
pub const GPUI_EVENT_OTHER: u32 = 0;
/// A key going down.
pub const GPUI_EVENT_KEY_DOWN: u32 = 1;
/// A key coming up.
pub const GPUI_EVENT_KEY_UP: u32 = 2;

/// Modifier bits. The five GPUI itself distinguishes (`gpui::Modifiers`), in
/// the order that struct declares them, so the mapping is a transcription
/// rather than a decision.
pub const GPUI_MOD_CONTROL: u32 = 1 << 0;
pub const GPUI_MOD_ALT: u32 = 1 << 1;
pub const GPUI_MOD_SHIFT: u32 = 1 << 2;
pub const GPUI_MOD_PLATFORM: u32 = 1 << 3;
pub const GPUI_MOD_FUNCTION: u32 = 1 << 4;

/// What a dispatched event hands its listener.
///
/// `#[repr(C)]` and passed by pointer. The pointer is valid only for the
/// duration of the callback; a handler that needs the key afterwards copies
/// it. `key` is a NUL-terminated UTF-8 string or null.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct GpuiEventPayload {
    /// One of `GPUI_EVENT_*`.
    pub kind: u32,
    /// `GPUI_MOD_*` bits.
    pub modifiers: u32,
    /// The base key name, unmodified. Null when the event carries no key.
    pub key: *const c_char,
    /// 1 when the platform reported an auto-repeat.
    pub repeat: u8,
    /// Explicit tail padding, so the struct's layout is the same thing on
    /// both sides of the boundary rather than whatever each compiler chose.
    pub _reserved: [u8; 7],
}

impl GpuiEventPayload {
    /// Read the payload into an owned record. `None` for a null pointer,
    /// which is the honest encoding of "this event carried nothing" and is
    /// kept distinct from "it carried an empty key".
    ///
    /// # Safety
    /// `p` must be null or point at a live `GpuiEventPayload` whose `key` is
    /// null or a NUL-terminated string.
    pub unsafe fn read(p: *const GpuiEventPayload) -> Option<(u32, String, u32, bool)> {
        if p.is_null() {
            return None;
        }
        let v = *p;
        let key = if v.key.is_null() {
            String::new()
        } else {
            cstr_to_str(v.key).to_string()
        };
        Some((v.kind, key, v.modifiers, v.repeat != 0))
    }
}

/// **The C layout of `GpuiEventPayload`, as the RUST compiler computed it.**
///
/// A struct passed by pointer across an FFI boundary is a contract that does
/// not fail to LINK when it drifts — it delivers a different key, silently,
/// which is `Verification-Harness-Traps.md` §25 one level down from the
/// helper it names. So the layout is readable, and `tests/test_bindings.nim`
/// compares Nim's own `sizeof` / `offsetof` against these numbers rather than
/// against a literal both sides could be wrong about together.
///
/// `field`: 0 = size, 1 = align, 2..5 = offset of `kind`, `modifiers`, `key`,
/// `repeat`. Anything else answers `u64::MAX`, so a caller that asked a
/// question this function does not understand gets an answer it cannot
/// mistake for a byte count.
#[no_mangle]
pub extern "C" fn gpui_event_payload_layout(field: u32) -> u64 {
    match field {
        0 => std::mem::size_of::<GpuiEventPayload>() as u64,
        1 => std::mem::align_of::<GpuiEventPayload>() as u64,
        2 => std::mem::offset_of!(GpuiEventPayload, kind) as u64,
        3 => std::mem::offset_of!(GpuiEventPayload, modifiers) as u64,
        4 => std::mem::offset_of!(GpuiEventPayload, key) as u64,
        5 => std::mem::offset_of!(GpuiEventPayload, repeat) as u64,
        _ => u64::MAX,
    }
}

/// Process-wide delivery sequence. Starts at 1 on the first delivery so that
/// a node reporting 0 means "nothing ever arrived" rather than "something
/// arrived first".
static DELIVERY_SEQ: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Delivery — the one routine
// ---------------------------------------------------------------------------

/// Deliver `event` to `node_id`, recording it in the element store and then
/// running the node's listeners. Returns the number of listeners reached.
///
/// The record is written **before** the callbacks and **while the tree lock
/// is held**; the callbacks run after it is released, because a callback that
/// mutates the tree through the FFI would otherwise deadlock (the property
/// `render_sync::test_dispatch_shadow_event_lock_release` already pins).
pub fn deliver(node_id: NodeId, event: &str, payload: Option<(u32, String, u32, bool)>) -> u32 {
    if node_id.is_null() {
        return 0;
    }
    let seq = DELIVERY_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    let (kind, key, modifiers, repeat) = match payload {
        Some((k, key, m, r)) => (k, key, m, r),
        None => (GPUI_EVENT_OTHER, String::new(), 0, false),
    };

    let listeners: Vec<(crate::EventCallback, i32)> = {
        let mut tree = lock_tree();
        let Some(n) = tree.get_mut(node_id) else {
            return 0;
        };
        let ls: Vec<(crate::EventCallback, i32)> = n
            .event_listeners
            .get(event)
            .map(|ls| ls.iter().map(|l| (l.callback, l.callback_id)).collect())
            .unwrap_or_default();
        // Recorded whether or not anything listens. "The key arrived at this
        // element" and "the key reached a handler" are different claims, and
        // PLAT-38's gate needs to tell them apart: `last_event` answers the
        // first, `delivery_count` the second.
        n.last_event = Some(DeliveredEvent {
            event: event.to_string(),
            kind,
            key,
            modifiers,
            repeat,
            seq,
        });
        n.delivery_count += ls.len() as u64;
        ls
    };

    let dispatcher = {
        crate::EVENT_DISPATCHER
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    };

    // The payload handed across the boundary is rebuilt from the RECORD, not
    // from the caller's struct, so the bytes a handler sees and the bytes the
    // store kept are the same bytes by construction. A handler comparing the
    // two would otherwise be comparing two independent copies and could not
    // see one of them going wrong.
    let (kind, key_c, modifiers, repeat) = {
        let tree = lock_tree();
        match tree.get(node_id).and_then(|n| n.last_event.as_ref()) {
            Some(rec) => (
                rec.kind,
                std::ffi::CString::new(rec.key.as_str()).unwrap_or_default(),
                rec.modifiers,
                rec.repeat,
            ),
            None => (
                GPUI_EVENT_OTHER,
                std::ffi::CString::default(),
                0u32,
                false,
            ),
        }
    };

    let has_payload = kind != GPUI_EVENT_OTHER || !key_c.as_bytes().is_empty() || modifiers != 0;
    let pay = GpuiEventPayload {
        kind,
        modifiers,
        key: if key_c.as_bytes().is_empty() {
            std::ptr::null()
        } else {
            key_c.as_ptr()
        },
        repeat: u8::from(repeat),
        _reserved: [0; 7],
    };
    let pay_ptr: *const GpuiEventPayload = if has_payload {
        &pay as *const GpuiEventPayload
    } else {
        std::ptr::null()
    };

    let mut reached = 0u32;
    for (cb, id) in listeners {
        if id > 0 {
            if let Some(dispatch) = dispatcher {
                dispatch(id, pay_ptr);
                reached += 1;
            }
        } else {
            cb(pay_ptr);
            reached += 1;
        }
    }
    // `key_c` must outlive the loop; naming it here makes that explicit to a
    // reader rather than leaving it to drop order.
    drop(key_c);
    reached
}

/// Dispatch `event` to `node` carrying `payload`. Returns the number of
/// listeners it reached.
///
/// **THE RETURN VALUE IS THE POINT OF THE SIGNATURE.** PLAT-38's gate: *"a
/// key that reaches nothing must be distinguishable from a key that reaches
/// everything"* — both satisfy "the event fired", and a `void` dispatch
/// cannot tell them apart.
#[no_mangle]
pub extern "C" fn gpui_dispatch_event_with(
    node: *mut GpuiElement,
    event: *const c_char,
    payload: *const GpuiEventPayload,
) -> u32 {
    let node_id = unsafe { handle_to_node_id(node) };
    if node_id.is_null() {
        return 0;
    }
    let event_str = unsafe { cstr_to_str(event) };
    let p = unsafe { GpuiEventPayload::read(payload) };
    deliver(node_id, event_str, p)
}

// ---------------------------------------------------------------------------
// Reading the element store back
// ---------------------------------------------------------------------------

/// The two-call sizing protocol this shim already uses for
/// `gpui_get_attribute`, `gpui_get_tag` and `gpui_get_text_content`: call
/// with a null buffer to learn the byte count, then again with a buffer.
///
/// It exists as a function here rather than as a fourth hand-copy of the same
/// eight lines (§30a — *"the worst instance was a whole re-derived module"*).
/// The three existing copies are left alone on purpose: rewriting them is a
/// change to functions this milestone does not otherwise touch, and would put
/// PLAT-38's diff on top of code another milestone is editing in the same
/// window. That is a residual, stated rather than closed.
fn write_str_out(s: &str, buf: *mut u8, buf_len: u64) -> u64 {
    let bytes = s.as_bytes();
    let needed = bytes.len() as u64;
    if buf.is_null() || buf_len == 0 {
        return needed;
    }
    let to_copy = std::cmp::min(bytes.len(), (buf_len - 1) as usize);
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, to_copy);
        *buf.add(to_copy) = 0;
    }
    needed
}

/// The base key name of the last event delivered to `node`, written into
/// `buf`. Returns the byte count (0 for "no key" and for "nothing arrived" —
/// use `gpui_last_event_seq` to tell those apart).
#[no_mangle]
pub extern "C" fn gpui_last_event_key(node: *mut GpuiElement, buf: *mut u8, buf_len: u64) -> u64 {
    let node_id = unsafe { handle_to_node_id(node) };
    let tree = lock_tree();
    let Some(rec) = tree.get(node_id).and_then(|n| n.last_event.as_ref()) else {
        return 0;
    };
    write_str_out(&rec.key, buf, buf_len)
}

/// The event NAME the last delivery arrived under.
#[no_mangle]
pub extern "C" fn gpui_last_event_name(node: *mut GpuiElement, buf: *mut u8, buf_len: u64) -> u64 {
    let node_id = unsafe { handle_to_node_id(node) };
    let tree = lock_tree();
    let Some(rec) = tree.get(node_id).and_then(|n| n.last_event.as_ref()) else {
        return 0;
    };
    write_str_out(&rec.event, buf, buf_len)
}

/// The modifier bitmask of the last event delivered to `node`.
#[no_mangle]
pub extern "C" fn gpui_last_event_modifiers(node: *mut GpuiElement) -> u32 {
    let node_id = unsafe { handle_to_node_id(node) };
    let tree = lock_tree();
    tree.get(node_id)
        .and_then(|n| n.last_event.as_ref())
        .map(|r| r.modifiers)
        .unwrap_or(0)
}

/// The kind (`GPUI_EVENT_*`) of the last event delivered to `node`.
#[no_mangle]
pub extern "C" fn gpui_last_event_kind(node: *mut GpuiElement) -> u32 {
    let node_id = unsafe { handle_to_node_id(node) };
    let tree = lock_tree();
    tree.get(node_id)
        .and_then(|n| n.last_event.as_ref())
        .map(|r| r.kind)
        .unwrap_or(GPUI_EVENT_OTHER)
}

/// 1 if the last event delivered to `node` was an auto-repeat.
#[no_mangle]
pub extern "C" fn gpui_last_event_repeat(node: *mut GpuiElement) -> u8 {
    let node_id = unsafe { handle_to_node_id(node) };
    let tree = lock_tree();
    u8::from(
        tree.get(node_id)
            .and_then(|n| n.last_event.as_ref())
            .map(|r| r.repeat)
            .unwrap_or(false),
    )
}

/// The process-wide sequence number of the last delivery to `node`, or 0 if
/// nothing ever arrived. **This is what makes "nothing arrived" and "the same
/// thing arrived twice" different observations**, which a comparison of key
/// names alone cannot do.
#[no_mangle]
pub extern "C" fn gpui_last_event_seq(node: *mut GpuiElement) -> u64 {
    let node_id = unsafe { handle_to_node_id(node) };
    let tree = lock_tree();
    tree.get(node_id)
        .and_then(|n| n.last_event.as_ref())
        .map(|r| r.seq)
        .unwrap_or(0)
}

/// How many listeners the deliveries to `node` have reached in total.
#[no_mangle]
pub extern "C" fn gpui_event_delivery_count(node: *mut GpuiElement) -> u64 {
    let node_id = unsafe { handle_to_node_id(node) };
    let tree = lock_tree();
    tree.get(node_id).map(|n| n.delivery_count).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Element focus
// ---------------------------------------------------------------------------

/// Declare whether `node` can take focus. The focus ORDER is the pre-order
/// walk of the elements carrying this, i.e. document order.
#[no_mangle]
pub extern "C" fn gpui_set_focusable(node: *mut GpuiElement, focusable: u8) {
    let node_id = unsafe { handle_to_node_id(node) };
    let mut tree = lock_tree();
    if let Some(n) = tree.get_mut(node_id) {
        n.focusable = focusable != 0;
        if !n.focusable {
            n.focused = false;
        }
    }
}

/// 1 if `node` is declared focusable.
#[no_mangle]
pub extern "C" fn gpui_is_focusable(node: *mut GpuiElement) -> u8 {
    let node_id = unsafe { handle_to_node_id(node) };
    let tree = lock_tree();
    u8::from(tree.get(node_id).map(|n| n.focusable).unwrap_or(false))
}

/// Give focus to `node`, exclusively. Returns 1 if it took it, 0 if the
/// request was REFUSED.
///
/// A request is refused when the node is not focusable, or when a focus trap
/// is active and the node is outside it. Refusing rather than silently
/// focusing is what makes `Modal` mean something: the entry's specified
/// behaviour is *exclusive input until dismissed*, and a trap that could be
/// escaped by asking is not one.
#[no_mangle]
pub extern "C" fn gpui_focus_element(node: *mut GpuiElement) -> u8 {
    let node_id = unsafe { handle_to_node_id(node) };
    if node_id.is_null() {
        return 0;
    }
    let mut tree = lock_tree();
    if !tree.get(node_id).map(|n| n.focusable).unwrap_or(false) {
        return 0;
    }
    let trap = tree.focus_trap_ids().last().copied().unwrap_or(NodeId::NULL);
    if !trap.is_null() && !tree.is_within(node_id, trap) {
        return 0;
    }
    tree.focus_only(node_id);
    1
}

/// Remove focus from `node` (and from nothing else).
#[no_mangle]
pub extern "C" fn gpui_blur_element(node: *mut GpuiElement) {
    let node_id = unsafe { handle_to_node_id(node) };
    let mut tree = lock_tree();
    if let Some(n) = tree.get_mut(node_id) {
        n.focused = false;
    }
}

/// 1 if `node` currently holds focus.
#[no_mangle]
pub extern "C" fn gpui_is_focused(node: *mut GpuiElement) -> u8 {
    let node_id = unsafe { handle_to_node_id(node) };
    let tree = lock_tree();
    u8::from(tree.get(node_id).map(|n| n.focused).unwrap_or(false))
}

/// A fresh handle to the focused element, or null.
#[no_mangle]
pub extern "C" fn gpui_focused_element() -> *mut GpuiElement {
    let tree = lock_tree();
    match tree.focused_ids().first() {
        Some(id) => node_id_to_handle(*id),
        None => std::ptr::null_mut(),
    }
}

/// **How many elements in the store hold focus.** The focus partition law
/// reads this and asserts it is 0 or 1, over the WHOLE tree rather than over
/// the element the case just focused. It walks every node; see
/// `tree::Node::focused` for why that is not an accident.
#[no_mangle]
pub extern "C" fn gpui_focused_count() -> u64 {
    let tree = lock_tree();
    tree.focused_ids().len() as u64
}

/// Declare `node` a focus trap (1) or stop trapping (0). Returns 1 on
/// success. Setting a trap moves focus to the first focusable element inside
/// it if the current holder is outside.
#[no_mangle]
pub extern "C" fn gpui_set_focus_trap(node: *mut GpuiElement, trap: u8) -> u8 {
    let node_id = unsafe { handle_to_node_id(node) };
    if node_id.is_null() {
        return 0;
    }
    let mut tree = lock_tree();
    if tree.get(node_id).is_none() {
        return 0;
    }
    if let Some(n) = tree.get_mut(node_id) {
        n.focus_trap = trap != 0;
    }
    if trap != 0 {
        let holder = tree.focused_ids().first().copied().unwrap_or(NodeId::NULL);
        if holder.is_null() || !tree.is_within(holder, node_id) {
            let inside = tree.focus_order(node_id);
            match inside.first() {
                Some(first) => tree.focus_only(*first),
                None => tree.blur_all(),
            }
        }
    }
    1
}

/// A fresh handle to the innermost active focus trap, or null.
#[no_mangle]
pub extern "C" fn gpui_focus_trap_element() -> *mut GpuiElement {
    let tree = lock_tree();
    match tree.focus_trap_ids().last() {
        Some(id) => node_id_to_handle(*id),
        None => std::ptr::null_mut(),
    }
}

/// The root the focus order is walked from: the declared root element, or —
/// when none has been declared — the lowest-id parentless node, so a test
/// that never called `gpui_set_root_element` still gets a total order rather
/// than an empty one.
fn focus_walk_root(tree: &crate::tree::Tree) -> NodeId {
    let declared = *crate::ROOT_NODE_ID.lock().unwrap_or_else(|p| p.into_inner());
    if !declared.is_null() && tree.get(declared).is_some() {
        return declared;
    }
    tree.root_ids().first().copied().unwrap_or(NodeId::NULL)
}

/// How many focusable elements are in the current focus order.
#[no_mangle]
pub extern "C" fn gpui_focusable_count() -> u64 {
    let tree = lock_tree();
    let root = focus_walk_root(&tree);
    let trap = tree.focus_trap_ids().last().copied().unwrap_or(NodeId::NULL);
    let from = if trap.is_null() { root } else { trap };
    tree.focus_order(from).len() as u64
}

/// The `index`-th element of the declared focus order, or null.
///
/// **The order is readable from the Rust side on purpose.** PLAT-35 filed
/// `PLAT35-VG4` because *"GPUI's order is declared by the leaf renderer and
/// enforced by nothing"*; an order that only the caller can see is one no
/// gate can check.
#[no_mangle]
pub extern "C" fn gpui_focusable_at(index: u64) -> *mut GpuiElement {
    let tree = lock_tree();
    let root = focus_walk_root(&tree);
    let trap = tree.focus_trap_ids().last().copied().unwrap_or(NodeId::NULL);
    let from = if trap.is_null() { root } else { trap };
    let order = tree.focus_order(from);
    match order.get(index as usize) {
        Some(id) => node_id_to_handle(*id),
        None => std::ptr::null_mut(),
    }
}

fn move_focus(delta: i64) -> u8 {
    let mut tree = lock_tree();
    let root = focus_walk_root(&tree);
    let trap = tree.focus_trap_ids().last().copied().unwrap_or(NodeId::NULL);
    let from = if trap.is_null() { root } else { trap };
    let order = tree.focus_order(from);
    if order.is_empty() {
        return 0;
    }
    let current = tree.focused_ids().first().copied().unwrap_or(NodeId::NULL);
    let next = match order.iter().position(|id| *id == current) {
        Some(pos) => {
            let n = order.len() as i64;
            let raw = pos as i64 + delta;
            // Wrapping rather than clamping: a clamp is a silent repair
            // (§36a) and would make "forward from the last element" and
            // "forward from the second-to-last" the same observation.
            order[(((raw % n) + n) % n) as usize]
        }
        None => {
            if delta >= 0 {
                order[0]
            } else {
                order[order.len() - 1]
            }
        }
    };
    tree.focus_only(next);
    1
}

/// Move focus to the next element in the declared order, wrapping.
#[no_mangle]
pub extern "C" fn gpui_focus_next() -> u8 {
    move_focus(1)
}

/// Move focus to the previous element in the declared order, wrapping.
#[no_mangle]
pub extern "C" fn gpui_focus_prev() -> u8 {
    move_focus(-1)
}

/// Deliver a key to whatever element holds focus. Returns the number of
/// listeners reached; 0 when nothing holds focus, when no window holds focus,
/// or when the focused element listens for nothing under that name.
///
/// **THE WINDOW-FOCUS GATE IS THE NEGATIVE TWIN.** PLAT-38 asks for *"the
/// same key delivered while the window does not hold focus changes
/// nothing"*, and a renderer that routed keys regardless of window focus
/// would satisfy every positive case in the suite. `window::any_window_focused`
/// is the one predicate both this and the assertion read.
#[no_mangle]
pub extern "C" fn gpui_dispatch_key_to_focus(
    event: *const c_char,
    payload: *const GpuiEventPayload,
) -> u32 {
    let event_str = unsafe { cstr_to_str(event) };
    let p = unsafe { GpuiEventPayload::read(payload) };
    deliver_key_to_focus(event_str, p)
}

/// The Rust-side spelling of the routine above.
///
/// **`gpui_app.rs`'s real `on_key_down` handler calls THIS, and the C entry
/// point calls it too.** A second routing routine for the compositor path
/// would be §30 in the exact place PLAT-38's gate reads — the suite would be
/// grading the C path while the window ran a different one.
pub fn deliver_key_to_focus(event: &str, payload: Option<(u32, String, u32, bool)>) -> u32 {
    if !crate::window::any_window_focused() {
        return 0;
    }
    let target = {
        let tree = lock_tree();
        tree.focused_ids().first().copied().unwrap_or(NodeId::NULL)
    };
    if target.is_null() {
        return 0;
    }
    let reached = deliver(target, event, payload);
    if reached > 0 && event == "keydown" {
        // The host's handler has run (and mutated the tree); the next frame
        // is the one that shows it.
        crate::frame_stats::note_key();
    }
    reached
}

/// The key string a keystroke is delivered with: GPUI's `key`, or — for a
/// single PRINTABLE key typed with no command modifier — GPUI's `key_char`,
/// the character the keystroke actually produced.
///
/// **BOTH ARE GPUI'S OWN SPELLINGS; THE SHIM STILL INVENTS NONE.** `key` is
/// the character printed on the key (`"q"` for Shift+q, and for q with Caps
/// Lock on) and `key_char` is what the keystroke typed (`"Q"`). Delivering
/// `key` alone lost every capital that arrived WITHOUT the Shift modifier —
/// Caps Lock, an input method, a synthetic keyboard (`wtype` typing a
/// keysym) — measured in PLAT-44's window lane, where `Q` was inserted as
/// `q` and Kakoune's `C` (copy selection below) ran as `c` (change). With the
/// Shift modifier the consumer's decoder already turned `q` + shift into
/// `Q`; passing `"Q"` with or without shift decodes to the same `Q`, so no
/// existing binding moves.
///
/// Kept to what cannot collide with a NAMED key: `key` must be one
/// character (so `"space"`, `"f10"`, `"enter"` are never replaced — `key_char`
/// for space is `" "`), `key_char` must be one printable ASCII character
/// other than space (the consumer's vocabulary is ASCII), and a command
/// modifier (control, alt, platform) keeps `key`, because a chord names the
/// KEY (`Ctrl+s`, not `Ctrl+\u{13}`).
pub fn key_for_consumer(key: &str, key_char: Option<&str>, command: bool) -> String {
    if !command && key.chars().count() == 1 {
        if let Some(c) = key_char {
            let mut chars = c.chars();
            if let (Some(ch), None) = (chars.next(), chars.next()) {
                if ch.is_ascii_graphic() {
                    return c.to_string();
                }
            }
        }
    }
    key.to_string()
}

/// The modifier bitmask for a GPUI `Modifiers`.
///
/// **THE SHIM DOES NOT RENAME THE KEY.** GPUI's own `keystroke.key` is
/// passed through verbatim (`"up"`, `"f10"`, `"a"`, `"space"`), and the
/// canonical-name vocabulary — `"Up"`, `"F10"`, `"Space"` — belongs to the
/// consumer that has one. A renderer that invented a third key vocabulary
/// would make the consumer's decoder and the renderer's decoder agree by
/// construction, and PLAT-38's key-identity law needs them to be two
/// independent readings of one keystroke.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
pub fn modifier_bits(m: &gpui::Modifiers) -> u32 {
    let mut bits = 0;
    if m.control {
        bits |= GPUI_MOD_CONTROL;
    }
    if m.alt {
        bits |= GPUI_MOD_ALT;
    }
    if m.shift {
        bits |= GPUI_MOD_SHIFT;
    }
    if m.platform {
        bits |= GPUI_MOD_PLATFORM;
    }
    if m.function {
        bits |= GPUI_MOD_FUNCTION;
    }
    bits
}

#[cfg(test)]
// The `unsafe` blocks below wrap calls that are safe `extern "C"` functions,
// so rustc reports them as unnecessary. They are kept because every other
// test module in this crate brackets shim calls the same way, and a module
// that looked different would read as a different KIND of test rather than as
// a lint difference.
#[allow(unused_unsafe)]
mod tests {
    #[test]
    fn test_key_for_consumer_delivers_the_typed_character() {
        use super::key_for_consumer;
        // Shift+q, and q under Caps Lock / a keysym keyboard: both `Q`.
        assert_eq!(key_for_consumer("q", Some("Q"), false), "Q");
        // A shifted symbol: the typed character.
        assert_eq!(key_for_consumer("7", Some("&"), false), "&");
        // Unchanged: plain letters, named keys, chords, and no key_char.
        assert_eq!(key_for_consumer("a", Some("a"), false), "a");
        assert_eq!(key_for_consumer("space", Some(" "), false), "space");
        assert_eq!(key_for_consumer("f10", None, false), "f10");
        assert_eq!(key_for_consumer("enter", Some("\n"), false), "enter");
        assert_eq!(key_for_consumer("s", Some("\u{13}"), true), "s");
        assert_eq!(key_for_consumer("s", Some("s"), true), "s");
        assert_eq!(key_for_consumer("q", None, false), "q");
        // Non-ASCII typed characters keep the key (the consumer's vocabulary
        // is ASCII), and so does a whitespace key_char on a one-char key.
        assert_eq!(key_for_consumer("q", Some("ๆ"), false), "q");
        assert_eq!(key_for_consumer("x", Some(" "), false), "x");
    }

    use super::*;
    use crate::{
        gpui_append_child, gpui_create_element, gpui_reset_tree, gpui_set_root_element,
        gpui_add_event_listener,
    };
    use serial_test::serial;
    use std::ffi::CString;
    use std::sync::atomic::{AtomicU32, Ordering as TestOrdering};

    fn c(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    static SEEN_MODS: AtomicU32 = AtomicU32::new(0xFFFF_FFFF);

    extern "C" fn record_mods(p: *const GpuiEventPayload) {
        let mods = unsafe { GpuiEventPayload::read(p) }
            .map(|(_, _, m, _)| m)
            .unwrap_or(0xDEAD_BEEF);
        SEEN_MODS.store(mods, TestOrdering::SeqCst);
    }

    /// The payload a LISTENER sees is the payload the ELEMENT STORE kept.
    ///
    /// Not two assertions about one value: the handler's copy comes back
    /// across a function pointer and the store's comes back through
    /// `gpui_last_event_modifiers`, which are different routes to the same
    /// record. A dispatcher that handed the handler a fresh zeroed struct
    /// would pass a store-only check.
    #[test]
    #[serial]
    fn payload_reaches_the_listener_and_the_store() {
        unsafe {
            gpui_reset_tree();
            SEEN_MODS.store(0xFFFF_FFFF, TestOrdering::SeqCst);
            let el = gpui_create_element(c("button").as_ptr());
            gpui_add_event_listener(el, c("keydown").as_ptr(), record_mods);
            let key = c("F10");
            let pay = GpuiEventPayload {
                kind: GPUI_EVENT_KEY_DOWN,
                modifiers: GPUI_MOD_SHIFT | GPUI_MOD_CONTROL,
                key: key.as_ptr(),
                repeat: 1,
                _reserved: [0; 7],
            };
            let reached = gpui_dispatch_event_with(el, c("keydown").as_ptr(), &pay);
            assert_eq!(reached, 1);
            assert_eq!(
                SEEN_MODS.load(TestOrdering::SeqCst),
                GPUI_MOD_SHIFT | GPUI_MOD_CONTROL
            );
            assert_eq!(
                gpui_last_event_modifiers(el),
                GPUI_MOD_SHIFT | GPUI_MOD_CONTROL
            );
            let mut buf = [0u8; 32];
            let n = gpui_last_event_key(el, buf.as_mut_ptr(), buf.len() as u64);
            assert_eq!(std::str::from_utf8(&buf[..n as usize]).unwrap(), "F10");
            assert_eq!(gpui_last_event_repeat(el), 1);
            assert!(gpui_last_event_seq(el) > 0);
        }
    }

    /// A key that reaches NOTHING is distinguishable from one that reaches a
    /// listener: the store records the arrival either way, and
    /// `delivery_count` is what separates them.
    #[test]
    #[serial]
    fn a_key_that_reaches_nothing_is_not_a_key_that_reached_something() {
        unsafe {
            gpui_reset_tree();
            let el = gpui_create_element(c("div").as_ptr());
            let key = c("Up");
            let pay = GpuiEventPayload {
                kind: GPUI_EVENT_KEY_DOWN,
                modifiers: 0,
                key: key.as_ptr(),
                repeat: 0,
                _reserved: [0; 7],
            };
            assert_eq!(gpui_dispatch_event_with(el, c("keydown").as_ptr(), &pay), 0);
            assert_eq!(gpui_event_delivery_count(el), 0);
            assert!(gpui_last_event_seq(el) > 0, "the arrival is still recorded");
        }
    }

    /// Focus is exclusive, and the count is taken over the whole store.
    #[test]
    #[serial]
    fn focus_is_exclusive_over_the_whole_store() {
        unsafe {
            gpui_reset_tree();
            let root = gpui_create_element(c("div").as_ptr());
            let a = gpui_create_element(c("button").as_ptr());
            let b = gpui_create_element(c("button").as_ptr());
            gpui_append_child(root, a);
            gpui_append_child(root, b);
            gpui_set_root_element(root);
            gpui_set_focusable(a, 1);
            gpui_set_focusable(b, 1);
            assert_eq!(gpui_focused_count(), 0);
            assert_eq!(gpui_focus_element(a), 1);
            assert_eq!(gpui_focused_count(), 1);
            assert_eq!(gpui_is_focused(a), 1);
            assert_eq!(gpui_focus_element(b), 1);
            assert_eq!(gpui_focused_count(), 1);
            assert_eq!(gpui_is_focused(a), 0);
            assert_eq!(gpui_is_focused(b), 1);
        }
    }

    /// A non-focusable element REFUSES focus rather than taking it quietly.
    #[test]
    #[serial]
    fn focus_is_refused_where_it_was_not_declared() {
        unsafe {
            gpui_reset_tree();
            let el = gpui_create_element(c("div").as_ptr());
            gpui_set_root_element(el);
            assert_eq!(gpui_focus_element(el), 0);
            assert_eq!(gpui_focused_count(), 0);
        }
    }

    /// The declared order is document order, forward and backward wrap, and
    /// a trap confines both.
    #[test]
    #[serial]
    fn focus_moves_through_the_declared_order_and_a_trap_confines_it() {
        unsafe {
            gpui_reset_tree();
            let root = gpui_create_element(c("div").as_ptr());
            let a = gpui_create_element(c("button").as_ptr());
            let modal = gpui_create_element(c("dialog").as_ptr());
            let m1 = gpui_create_element(c("button").as_ptr());
            let m2 = gpui_create_element(c("button").as_ptr());
            gpui_append_child(root, a);
            gpui_append_child(root, modal);
            gpui_append_child(modal, m1);
            gpui_append_child(modal, m2);
            gpui_set_root_element(root);
            for el in [a, m1, m2] {
                gpui_set_focusable(el, 1);
            }
            assert_eq!(gpui_focusable_count(), 3);
            assert_eq!(gpui_focus_element(a), 1);
            assert_eq!(gpui_focus_next(), 1);
            assert_eq!(gpui_is_focused(m1), 1);
            assert_eq!(gpui_focus_prev(), 1);
            assert_eq!(gpui_is_focused(a), 1);
            // Backward from the first wraps to the last.
            assert_eq!(gpui_focus_prev(), 1);
            assert_eq!(gpui_is_focused(m2), 1);

            // The trap: the order shrinks, and the element outside refuses.
            assert_eq!(gpui_set_focus_trap(modal, 1), 1);
            assert_eq!(gpui_focusable_count(), 2);
            assert_eq!(gpui_focus_element(a), 0, "outside the trap");
            assert_eq!(gpui_is_focused(a), 0);
            assert_eq!(gpui_focus_next(), 1);
            assert_eq!(gpui_is_focused(m1), 1, "wrapped inside the trap");
        }
    }

    /// Setting a trap while focus sits outside it MOVES focus inside.
    #[test]
    #[serial]
    fn a_trap_takes_focus_from_outside_itself() {
        unsafe {
            gpui_reset_tree();
            let root = gpui_create_element(c("div").as_ptr());
            let a = gpui_create_element(c("button").as_ptr());
            let modal = gpui_create_element(c("dialog").as_ptr());
            let m1 = gpui_create_element(c("button").as_ptr());
            gpui_append_child(root, a);
            gpui_append_child(root, modal);
            gpui_append_child(modal, m1);
            gpui_set_root_element(root);
            gpui_set_focusable(a, 1);
            gpui_set_focusable(m1, 1);
            assert_eq!(gpui_focus_element(a), 1);
            assert_eq!(gpui_set_focus_trap(modal, 1), 1);
            assert_eq!(gpui_is_focused(a), 0);
            assert_eq!(gpui_is_focused(m1), 1);
            assert_eq!(gpui_focused_count(), 1);
        }
    }
}
