//! C FFI shim for GPUI, exposing the 13 procs required by IsoNim's RendererBackend.
//!
//! Each function uses `extern "C"` with `#[no_mangle]` so Nim can link against the
//! compiled cdylib. Element handles are opaque pointers managed on the Rust side.
//!
//! ## Architecture
//!
//! GPUI is a hybrid immediate/retained mode GPU-accelerated UI framework. IsoNim
//! needs imperative tree manipulation. We bridge this by:
//!
//! 1. Maintaining a **shadow tree** of `Node` structs in Rust.
//! 2. Exposing the 13 RendererBackend operations as `extern "C"` functions that
//!    manipulate this tree imperatively.
//! 3. A separate render-sync step (M2+) will translate the shadow tree into
//!    GPUI's element model (div, text, img) for actual rendering.
//!
//! Element handles (`*mut GpuiElement`) are thin wrappers around `NodeId` values.
//! They are heap-allocated so the Nim side can hold them as opaque pointers.
//!
//! ## GPUI API Background
//!
//! GPUI's element model is declarative. Elements are created via builder functions:
//! - `div()` -> `Div` (general-purpose flexbox container, the workhorse)
//! - `img()` -> `Img` (image rendering with caching)
//! - `svg()` -> `Svg` (SVG rendering)
//! - `canvas()` -> `Canvas` (custom painting)
//! - Strings implement `IntoElement` for text content
//!
//! Children are added via `.child(elem)` / `.children(iter)` (ParentElement trait).
//! Styling uses Tailwind-inspired builder methods: `.bg()`, `.flex()`, `.w()`, etc.
//! Events use `.on_mouse_up()`, `.on_key_down()`, `.on_action()`, etc.
//!
//! State is managed through `Entity<T>` handles owned by the `App` context.
//! Views implement `Render` (stateful, `&mut self`) or `RenderOnce` (stateless,
//! consumes `self`). Re-renders are triggered by `cx.notify()`.
//!
//! Since GPUI has NO imperative DOM API (no appendChild, no removeChild), the
//! shadow tree is essential: IsoNim mutates it imperatively, and the render-sync
//! step translates it to GPUI's declarative builders each frame.

use std::ffi::CStr;
use std::os::raw::c_char;

// ---------------------------------------------------------------------------
// Shadow tree types
// ---------------------------------------------------------------------------

/// Unique identifier for a node in the shadow tree.
type NodeId = u64;

/// Opaque element handle exposed to Nim.
#[repr(C)]
pub struct GpuiElement {
    id: NodeId,
}

/// Event listener stored in the shadow tree.
struct EventListener {
    _event: String,
    _callback: extern "C" fn(),
}

/// A single node in the shadow tree.
struct Node {
    _id: NodeId,
    _tag: String,
    text: String,
    attributes: Vec<(String, String)>,
    styles: Vec<(String, String)>,
    events: Vec<EventListener>,
    children: Vec<NodeId>,
    parent: Option<NodeId>,
}

/// The global shadow tree.
struct Tree {
    nodes: Vec<Node>,
    next_id: NodeId,
}

impl Tree {
    fn new() -> Self {
        Tree {
            nodes: Vec::new(),
            next_id: 1,
        }
    }

    fn alloc_id(&mut self) -> NodeId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn add_node(&mut self, tag: String) -> NodeId {
        let id = self.alloc_id();
        self.nodes.push(Node {
            _id: id,
            _tag: tag,
            text: String::new(),
            attributes: Vec::new(),
            styles: Vec::new(),
            events: Vec::new(),
            children: Vec::new(),
            parent: None,
        });
        id
    }

    fn find(&self, id: NodeId) -> Option<usize> {
        self.nodes.iter().position(|n| n._id == id)
    }

    fn find_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n._id == id)
    }

    fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

use std::sync::Mutex;

static TREE: std::sync::LazyLock<Mutex<Tree>> =
    std::sync::LazyLock::new(|| Mutex::new(Tree::new()));

fn lock_tree() -> std::sync::MutexGuard<'static, Tree> {
    match TREE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// ---------------------------------------------------------------------------
// Helper: box a handle for FFI
// ---------------------------------------------------------------------------

fn box_handle(id: NodeId) -> *mut GpuiElement {
    Box::into_raw(Box::new(GpuiElement { id }))
}

fn unbox_id(ptr: *mut GpuiElement) -> Option<NodeId> {
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { &*ptr }.id)
    }
}

fn c_str_to_string(s: *const c_char) -> String {
    if s.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(s) }
        .to_string_lossy()
        .into_owned()
}

// ===========================================================================
// 13 RendererBackend extern "C" functions
// ===========================================================================

/// RendererBackend proc #1: createElement(tag) -> Element
///
/// **GPUI mapping:** During render-sync (M2+), the tag determines which GPUI
/// builder is used:
/// - "div" (and most container tags) -> `gpui::div()` (Div element)
/// - "img" -> `gpui::img()` (Img element with .uri() for source)
/// - "svg" -> `gpui::svg()` (Svg element with .path() for source)
/// - "text" tags -> `gpui::div()` with text child (GPUI has no standalone text element)
///
/// The Nim-side `renderer.nim` maps HTML tags to these GPUI names before
/// calling this function (e.g., "section" -> "div", "span" -> "text").
///
/// **Current behavior:** Allocates a Node in the shadow tree with the given tag.
/// No GPUI element is created yet -- that happens during render-sync.
#[no_mangle]
pub extern "C" fn gpui_create_element(tag: *const c_char) -> *mut GpuiElement {
    let tag = c_str_to_string(tag);
    let mut tree = lock_tree();
    let id = tree.add_node(tag);
    box_handle(id)
}

/// RendererBackend proc #2: createTextNode(text) -> Element
///
/// **GPUI mapping:** GPUI does not have standalone text elements. Text is added
/// as a child of a Div via `.child("string")` -- strings implement `IntoElement`.
/// For styled text, GPUI provides `StyledText` with `TextRun`s for rich formatting.
///
/// During render-sync, `#text` nodes are converted to string children of their
/// parent div rather than becoming independent GPUI elements.
///
/// **Current behavior:** Allocates a Node with tag="#text" and the given text content.
#[no_mangle]
pub extern "C" fn gpui_create_text_node(text: *const c_char) -> *mut GpuiElement {
    let text_str = c_str_to_string(text);
    let mut tree = lock_tree();
    let id = tree.add_node("#text".to_string());
    if let Some(node) = tree.find_mut(id) {
        node.text = text_str;
    }
    box_handle(id)
}

/// RendererBackend proc #3: appendChild(parent, child)
///
/// **GPUI mapping:** GPUI uses `.child(elem)` on the ParentElement trait to add
/// children during element construction. There is no runtime appendChild -- the
/// element tree is rebuilt each frame from `Render::render()`.
///
/// During render-sync, the shadow tree's children list determines the order of
/// `.child()` calls in the GPUI builder chain. This function just maintains the
/// shadow tree's parent-child relationships.
///
/// **Challenge:** GPUI children are added declaratively in a single builder chain.
/// The render-sync must iterate shadow tree children in order and emit
/// `.child(render_node(child))` for each.
#[no_mangle]
pub extern "C" fn gpui_append_child(parent: *mut GpuiElement, child: *mut GpuiElement) {
    let (Some(pid), Some(cid)) = (unbox_id(parent), unbox_id(child)) else {
        return;
    };
    let mut tree = lock_tree();
    // Remove from old parent if any
    if let Some(old_parent_id) = tree.find_mut(cid).and_then(|n| n.parent) {
        if let Some(old_parent) = tree.find_mut(old_parent_id) {
            old_parent.children.retain(|&id| id != cid);
        }
    }
    if let Some(parent_node) = tree.find_mut(pid) {
        parent_node.children.push(cid);
    }
    if let Some(child_node) = tree.find_mut(cid) {
        child_node.parent = Some(pid);
    }
}

/// RendererBackend proc #4: insertBefore(parent, child, reference)
///
/// **GPUI mapping:** Same as appendChild -- GPUI has no insertBefore. Child order
/// in the shadow tree determines the `.child()` call order during render-sync.
/// Inserting before a reference node just means the shadow tree children list
/// has the new child at the correct position.
///
/// **Current behavior:** Inserts child into parent.children before the reference
/// node (or appends if reference is null/not found).
#[no_mangle]
pub extern "C" fn gpui_insert_before(
    parent: *mut GpuiElement,
    child: *mut GpuiElement,
    reference: *mut GpuiElement,
) {
    let (Some(pid), Some(cid)) = (unbox_id(parent), unbox_id(child)) else {
        return;
    };
    let ref_id = unbox_id(reference);
    let mut tree = lock_tree();
    // Remove from old parent
    if let Some(old_parent_id) = tree.find_mut(cid).and_then(|n| n.parent) {
        if let Some(old_parent) = tree.find_mut(old_parent_id) {
            old_parent.children.retain(|&id| id != cid);
        }
    }
    if let Some(parent_node) = tree.find_mut(pid) {
        if let Some(rid) = ref_id {
            if let Some(pos) = parent_node.children.iter().position(|&id| id == rid) {
                parent_node.children.insert(pos, cid);
            } else {
                parent_node.children.push(cid);
            }
        } else {
            parent_node.children.push(cid);
        }
    }
    if let Some(child_node) = tree.find_mut(cid) {
        child_node.parent = Some(pid);
    }
}

/// RendererBackend proc #5: removeChild(parent, child)
///
/// **GPUI mapping:** GPUI has no removeChild. Removing a child from the shadow tree
/// means it will not appear in the `.child()` chain during the next render-sync.
/// The GPUI element simply ceases to exist in the next frame's element tree.
///
/// **Current behavior:** Removes child from parent.children and clears child.parent.
/// The shadow tree node is NOT deallocated (it may be re-parented later).
#[no_mangle]
pub extern "C" fn gpui_remove_child(parent: *mut GpuiElement, child: *mut GpuiElement) {
    let (Some(pid), Some(cid)) = (unbox_id(parent), unbox_id(child)) else {
        return;
    };
    let mut tree = lock_tree();
    if let Some(parent_node) = tree.find_mut(pid) {
        parent_node.children.retain(|&id| id != cid);
    }
    if let Some(child_node) = tree.find_mut(cid) {
        child_node.parent = None;
    }
}

/// RendererBackend proc #6: setAttribute(node, name, value)
///
/// **GPUI mapping:** GPUI elements don't have a generic setAttribute API.
/// Attributes map to specific builder methods during render-sync:
/// - "id" -> `.id(ElementId::from(value))` (for cross-frame identity tracking)
/// - "src" -> `.uri(value)` on Img elements
/// - "class" -> ignored (styling is inline via Styled trait methods)
/// - "placeholder", "value" -> stored as metadata for input-like elements
/// - "disabled" -> negated to "enabled" by the Nim-side mapper
///
/// Most HTML attributes have no direct GPUI equivalent. They are stored in the
/// shadow tree for potential use by the render-sync or for testing/inspection.
#[no_mangle]
pub extern "C" fn gpui_set_attribute(
    node: *mut GpuiElement,
    name: *const c_char,
    value: *const c_char,
) {
    let Some(nid) = unbox_id(node) else { return };
    let name = c_str_to_string(name);
    let value = c_str_to_string(value);
    let mut tree = lock_tree();
    if let Some(n) = tree.find_mut(nid) {
        if let Some(attr) = n.attributes.iter_mut().find(|(k, _)| k == &name) {
            attr.1 = value;
        } else {
            n.attributes.push((name, value));
        }
    }
}

/// RendererBackend proc #7: removeAttribute(node, name)
///
/// **GPUI mapping:** Removing an attribute from the shadow tree means the
/// corresponding GPUI builder method will not be called during the next
/// render-sync. For example, removing "id" means `.id()` won't be called.
#[no_mangle]
pub extern "C" fn gpui_remove_attribute(node: *mut GpuiElement, name: *const c_char) {
    let Some(nid) = unbox_id(node) else { return };
    let name = c_str_to_string(name);
    let mut tree = lock_tree();
    if let Some(n) = tree.find_mut(nid) {
        n.attributes.retain(|(k, _)| k != &name);
    }
}

/// RendererBackend proc #8: setTextContent(node, text)
///
/// **GPUI mapping:** Text content is rendered by passing a string to `.child()`:
/// `div().child("Hello world")`. For text-tagged shadow nodes, the render-sync
/// produces the text string as a child element. For container nodes with direct
/// text, it is appended as an additional `.child()`.
///
/// GPUI's `StyledText` with `TextRun`s can be used for rich text (multiple
/// styles within one text block), but that requires M3+ implementation.
#[no_mangle]
pub extern "C" fn gpui_set_text_content(node: *mut GpuiElement, text: *const c_char) {
    let Some(nid) = unbox_id(node) else { return };
    let text = c_str_to_string(text);
    let mut tree = lock_tree();
    if let Some(n) = tree.find_mut(nid) {
        n.text = text;
    }
}

/// RendererBackend proc #9: setStyle(node, prop, value)
///
/// **GPUI mapping:** GPUI uses the `Styled` trait with Tailwind-inspired builder
/// methods. The render-sync parses prop/value strings and calls the matching method:
///
/// - "width"/"height" -> `.w(px(...))` / `.h(px(...))`
/// - "bg"/"background-color" -> `.bg(rgb(...))`
/// - "text_color"/"color" -> `.text_color(rgb(...))`
/// - "padding"/"margin" -> `.p(px(...))` / `.m(px(...))`
/// - "flex_direction: row" -> `.flex_row()`
/// - "flex_direction: col" -> `.flex_col()`
/// - "align_items: center" -> `.items_center()`
/// - "justify_content: center" -> `.justify_center()`
/// - "gap" -> `.gap(px(...))`
/// - "corner_radius" -> `.rounded(px(...))`
/// - "border_color" -> `.border_color(rgb(...))`
/// - "shadow" -> `.shadow_lg()`
/// - "cursor: pointer" -> `.cursor_pointer()`
///
/// **Challenge:** GPUI expects typed values (Pixels, Hsla, enums), not CSS strings.
/// The render-sync needs parsers for dimensions ("16px" -> px(16.0)),
/// colors ("#ff0000" -> rgb(0xff0000)), and enum values.
///
/// The Nim-side `renderer.nim` pre-maps CSS property names to GPUI-friendly names
/// (e.g., "background-color" -> "bg") to simplify the Rust-side parsing.
#[no_mangle]
pub extern "C" fn gpui_set_style(
    node: *mut GpuiElement,
    prop: *const c_char,
    value: *const c_char,
) {
    let Some(nid) = unbox_id(node) else { return };
    let prop = c_str_to_string(prop);
    let value = c_str_to_string(value);
    let mut tree = lock_tree();
    if let Some(n) = tree.find_mut(nid) {
        if let Some(style) = n.styles.iter_mut().find(|(k, _)| k == &prop) {
            style.1 = value;
        } else {
            n.styles.push((prop, value));
        }
    }
}

/// RendererBackend proc #10: addEventListener(node, event, handler)
///
/// **GPUI mapping:** GPUI elements handle events via InteractiveElement trait methods:
/// - "click"/"mouseup" -> `.on_mouse_up(MouseButton::Left, cx.listener(...))`
/// - "mousedown" -> `.on_mouse_down(MouseButton::Left, cx.listener(...))`
/// - "mousemove" -> `.on_mouse_move(cx.listener(...))`
/// - "keydown" -> `.on_key_down(cx.listener(...))`
/// - "keyup" -> `.on_key_up(cx.listener(...))`
/// - "scroll" -> `.on_scroll_wheel(cx.listener(...))`
///
/// During render-sync, the shadow tree's event listeners are translated into
/// GPUI event handler registrations. The GPUI handler calls the stored
/// `extern "C" fn()` callback, which trampolines into Nim.
///
/// **Challenge:** The current callback type is `extern "C" fn()` with no event data.
/// For full event support (M3+), we need to pass mouse position, key codes, etc.
/// via a `*const EventData` parameter.
///
/// **Challenge:** GPUI event handlers require `cx.listener()` which captures the
/// view's Entity. The render-sync must have access to the view context to create
/// proper listener closures.
#[no_mangle]
pub extern "C" fn gpui_add_event_listener(
    node: *mut GpuiElement,
    event: *const c_char,
    handler: extern "C" fn(),
) {
    let Some(nid) = unbox_id(node) else { return };
    let event = c_str_to_string(event);
    let mut tree = lock_tree();
    if let Some(n) = tree.find_mut(nid) {
        n.events.push(EventListener {
            _event: event,
            _callback: handler,
        });
    }
}

/// RendererBackend proc #11: firstChild(node) -> Element
///
/// **GPUI mapping:** None. GPUI's declarative model has no tree traversal API.
/// This is purely a shadow tree operation used by IsoNim's VDOM diffing algorithm
/// to walk the existing tree and compare it against the new virtual tree.
///
/// Returns a new handle to the first child node, or null if no children exist.
#[no_mangle]
pub extern "C" fn gpui_first_child(node: *mut GpuiElement) -> *mut GpuiElement {
    let Some(nid) = unbox_id(node) else {
        return std::ptr::null_mut();
    };
    let tree = lock_tree();
    let idx = tree.find(nid);
    match idx {
        Some(i) => {
            if let Some(&first) = tree.nodes[i].children.first() {
                box_handle(first)
            } else {
                std::ptr::null_mut()
            }
        }
        None => std::ptr::null_mut(),
    }
}

/// RendererBackend proc #12: nextSibling(node) -> Element
///
/// **GPUI mapping:** None. Purely a shadow tree traversal operation for IsoNim's
/// VDOM diffing. Finds the node in its parent's children list and returns the
/// next sibling, or null if it is the last child.
#[no_mangle]
pub extern "C" fn gpui_next_sibling(node: *mut GpuiElement) -> *mut GpuiElement {
    let Some(nid) = unbox_id(node) else {
        return std::ptr::null_mut();
    };
    let tree = lock_tree();
    let parent_id = match tree.find(nid) {
        Some(i) => tree.nodes[i].parent,
        None => return std::ptr::null_mut(),
    };
    let Some(pid) = parent_id else {
        return std::ptr::null_mut();
    };
    let parent_idx = match tree.find(pid) {
        Some(i) => i,
        None => return std::ptr::null_mut(),
    };
    let children = &tree.nodes[parent_idx].children;
    if let Some(pos) = children.iter().position(|&id| id == nid) {
        if pos + 1 < children.len() {
            return box_handle(children[pos + 1]);
        }
    }
    std::ptr::null_mut()
}

/// RendererBackend proc #13: parentNode(node) -> Element
///
/// **GPUI mapping:** None. Purely a shadow tree traversal operation for IsoNim's
/// VDOM diffing. Returns the parent node handle, or null if the node is a root.
#[no_mangle]
pub extern "C" fn gpui_parent_node(node: *mut GpuiElement) -> *mut GpuiElement {
    let Some(nid) = unbox_id(node) else {
        return std::ptr::null_mut();
    };
    let tree = lock_tree();
    match tree.find(nid) {
        Some(i) => match tree.nodes[i].parent {
            Some(pid) => box_handle(pid),
            None => std::ptr::null_mut(),
        },
        None => std::ptr::null_mut(),
    }
}

// ===========================================================================
// Window / event loop management
// ===========================================================================

type RootBuilderCallback = extern "C" fn(*mut GpuiElement);

/// Launch the GPUI application with a window.
///
/// **GPUI mapping (M2+):** This will become:
/// ```ignore
/// Application::new().run(|cx: &mut App| {
///     let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
///     cx.open_window(WindowOptions {
///         window_bounds: Some(WindowBounds::Windowed(bounds)),
///         ..Default::default()
///     }, |_, cx| {
///         // Call root_builder to let Nim build the shadow tree
///         root_builder(root_handle);
///         // Create a NimRootView that renders from the shadow tree
///         cx.new(|_| NimRootView)
///     });
/// });
/// ```
///
/// `Application::new()` creates the GPUI app; `.run()` starts the event loop
/// (takes over the main thread). `cx.open_window()` creates a platform window
/// with the given options. The root view implements `Render` and translates
/// the shadow tree to GPUI elements each frame.
///
/// **Current behavior (stub):** Creates a root div node and calls the builder
/// callback without starting an actual GPUI event loop.
#[no_mangle]
pub extern "C" fn gpui_launch(
    _title: *const c_char,
    _width: f64,
    _height: f64,
    _root_builder: RootBuilderCallback,
) {
    // Stub: in gpui-backend mode this would create a GPUI Application and Window.
    // For now, create a root element and call the builder.
    let root = gpui_create_element(b"div\0".as_ptr() as *const c_char);
    _root_builder(root);
}

/// Dispatch a named event on a shadow tree node (for testing).
///
/// **GPUI mapping:** In a real GPUI app, events are dispatched by the platform
/// (mouse clicks, key presses) and routed through GPUI's hit-testing system to
/// the appropriate element's event handler. This function simulates that for
/// testing: it finds all EventListeners on the node matching the event name
/// and calls their callbacks.
///
/// **Note:** The tree lock is dropped before calling callbacks to avoid deadlock
/// if a callback modifies the tree (which is the normal case -- Nim event
/// handlers typically update UI state).
#[no_mangle]
pub extern "C" fn gpui_dispatch_event(node: *mut GpuiElement, event: *const c_char) {
    let Some(nid) = unbox_id(node) else { return };
    let event_name = c_str_to_string(event);
    let tree = lock_tree();
    if let Some(idx) = tree.find(nid) {
        let callbacks: Vec<extern "C" fn()> = tree.nodes[idx]
            .events
            .iter()
            .filter(|e| e._event == event_name)
            .map(|e| e._callback)
            .collect();
        drop(tree);
        for cb in callbacks {
            cb();
        }
    }
}

// ===========================================================================
// Memory management
// ===========================================================================

#[no_mangle]
pub extern "C" fn gpui_destroy_element(handle: *mut GpuiElement) {
    if !handle.is_null() {
        unsafe {
            let _ = Box::from_raw(handle);
        }
    }
}

// ===========================================================================
// Debugging / testing
// ===========================================================================

#[no_mangle]
pub extern "C" fn gpui_reset_tree() {
    let mut tree = lock_tree();
    *tree = Tree::new();
}

#[no_mangle]
pub extern "C" fn gpui_tree_node_count() -> u64 {
    let tree = lock_tree();
    tree.node_count() as u64
}

// ===========================================================================
// Tree inspection (for cross-renderer testing)
// ===========================================================================

#[no_mangle]
pub extern "C" fn gpui_child_count(node: *mut GpuiElement) -> u64 {
    let Some(nid) = unbox_id(node) else { return 0 };
    let tree = lock_tree();
    match tree.find(nid) {
        Some(i) => tree.nodes[i].children.len() as u64,
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn gpui_get_text_content(
    node: *mut GpuiElement,
    buf: *mut u8,
    buf_len: u64,
) -> u64 {
    let Some(nid) = unbox_id(node) else { return 0 };
    let tree = lock_tree();
    let text = match tree.find(nid) {
        Some(i) => tree.nodes[i].text.clone(),
        None => return 0,
    };
    let needed = text.len() as u64;
    if !buf.is_null() && buf_len > 0 {
        let copy_len = std::cmp::min(needed, buf_len - 1) as usize;
        unsafe {
            std::ptr::copy_nonoverlapping(text.as_ptr(), buf, copy_len);
            *buf.add(copy_len) = 0;
        }
    }
    needed
}

#[no_mangle]
pub extern "C" fn gpui_get_attribute(
    node: *mut GpuiElement,
    name: *const c_char,
    buf: *mut u8,
    buf_len: u64,
) -> u64 {
    let Some(nid) = unbox_id(node) else { return 0 };
    let attr_name = c_str_to_string(name);
    let tree = lock_tree();
    let value = match tree.find(nid) {
        Some(i) => {
            match tree.nodes[i]
                .attributes
                .iter()
                .find(|(k, _)| k == &attr_name)
            {
                Some((_, v)) => v.clone(),
                None => return 0,
            }
        }
        None => return 0,
    };
    let needed = value.len() as u64;
    if !buf.is_null() && buf_len > 0 {
        let copy_len = std::cmp::min(needed, buf_len - 1) as usize;
        unsafe {
            std::ptr::copy_nonoverlapping(value.as_ptr(), buf, copy_len);
            *buf.add(copy_len) = 0;
        }
    }
    needed
}

#[no_mangle]
pub extern "C" fn gpui_nth_child(node: *mut GpuiElement, index: u64) -> *mut GpuiElement {
    let Some(nid) = unbox_id(node) else {
        return std::ptr::null_mut();
    };
    let tree = lock_tree();
    match tree.find(nid) {
        Some(i) => {
            let children = &tree.nodes[i].children;
            if (index as usize) < children.len() {
                box_handle(children[index as usize])
            } else {
                std::ptr::null_mut()
            }
        }
        None => std::ptr::null_mut(),
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn reset() {
        gpui_reset_tree();
    }

    #[test]
    fn test_create_element() {
        reset();
        let el = gpui_create_element(b"div\0".as_ptr() as *const c_char);
        assert!(!el.is_null());
        assert_eq!(gpui_tree_node_count(), 1);
        gpui_destroy_element(el);
    }

    #[test]
    fn test_append_child() {
        reset();
        let parent = gpui_create_element(b"div\0".as_ptr() as *const c_char);
        let child = gpui_create_element(b"span\0".as_ptr() as *const c_char);
        gpui_append_child(parent, child);
        assert_eq!(gpui_child_count(parent), 1);
        gpui_destroy_element(child);
        gpui_destroy_element(parent);
    }

    #[test]
    fn test_set_text_content() {
        reset();
        let el = gpui_create_text_node(b"hello\0".as_ptr() as *const c_char);
        let mut buf = [0u8; 64];
        let len = gpui_get_text_content(el, buf.as_mut_ptr(), 64);
        assert_eq!(len, 5);
        let s = std::str::from_utf8(&buf[..5]).unwrap();
        assert_eq!(s, "hello");
        gpui_destroy_element(el);
    }
}
