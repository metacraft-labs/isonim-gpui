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
//! 1. Maintaining a **shadow tree** of `Node` structs in Rust (`tree` module).
//! 2. Exposing the 13 RendererBackend operations as `extern "C"` functions that
//!    manipulate this tree imperatively.
//! 3. A separate render-sync step (M4+) will translate the shadow tree into
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

// Some items are pub in submodules for future milestones (render-sync, etc.)
#[allow(dead_code)]
mod tree;
#[allow(dead_code)]
mod window;
mod render_sync;
#[cfg(feature = "gpui-backend")]
mod gpui_app;

use std::ffi::CStr;
use std::os::raw::c_char;
use std::sync::Mutex;

use tree::{EventListener, Node, NodeId, Tree};
use window::{CloseCallback, FocusCallback, ResizeCallback};

/// Global shadow tree protected by a mutex.
/// All extern "C" functions lock this to perform tree operations.
static TREE: std::sync::LazyLock<Mutex<Tree>> =
    std::sync::LazyLock::new(|| Mutex::new(Tree::new()));

/// Global root node ID for the render-sync bridge.
/// Set by `gpui_launch()` so the GPUI view knows which node is the root.
static ROOT_NODE_ID: std::sync::LazyLock<Mutex<NodeId>> =
    std::sync::LazyLock::new(|| Mutex::new(NodeId::NULL));

/// Lock the global tree, recovering from poison if needed.
/// Since the tree is always in a valid (if inconsistent) state after a panic,
/// we simply clear the poison and continue.
fn lock_tree() -> std::sync::MutexGuard<'static, Tree> {
    match TREE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Opaque handle to a GPUI element. Wraps a NodeId.
/// Allocated on the heap via Box so Nim holds a stable pointer.
#[repr(C)]
pub struct GpuiElement {
    node_id: u64,
}

/// Helper: convert a raw C string pointer to a Rust &str.
/// Returns "" if the pointer is null or not valid UTF-8.
unsafe fn cstr_to_str<'a>(ptr: *const c_char) -> &'a str {
    if ptr.is_null() {
        return "";
    }
    match CStr::from_ptr(ptr).to_str() {
        Ok(s) => s,
        Err(_) => "",
    }
}

/// Helper: allocate a GpuiElement handle on the heap for the given NodeId.
/// Returns null if the id is NULL.
fn node_id_to_handle(id: NodeId) -> *mut GpuiElement {
    if id.is_null() {
        return std::ptr::null_mut();
    }
    Box::into_raw(Box::new(GpuiElement { node_id: id.0 }))
}

/// Helper: extract NodeId from a handle pointer. Returns NodeId::NULL if null.
unsafe fn handle_to_node_id(handle: *mut GpuiElement) -> NodeId {
    if handle.is_null() {
        NodeId::NULL
    } else {
        NodeId((*handle).node_id)
    }
}

// ===========================================================================
// 13 RendererBackend extern "C" functions
// ===========================================================================

// ---------------------------------------------------------------------------
// 1. createElement
// ---------------------------------------------------------------------------

/// Create a new element with the given tag name.
///
/// Returns a heap-allocated handle that the caller (Nim) must hold.
/// The element is added to the global shadow tree but not attached to any parent.
///
/// **GPUI mapping:** During render-sync, the tag determines which GPUI builder
/// is used (div, img, svg, or text container). See `tree::tag_to_gpui_kind`.
#[no_mangle]
pub extern "C" fn gpui_create_element(tag: *const c_char) -> *mut GpuiElement {
    let tag_str = unsafe { cstr_to_str(tag) };
    let node = Node::new_element(tag_str);
    let mut tree = lock_tree();
    let id = tree.insert(node);
    node_id_to_handle(id)
}

// ---------------------------------------------------------------------------
// 2. createTextNode
// ---------------------------------------------------------------------------

/// Create a text node with the given content.
#[no_mangle]
pub extern "C" fn gpui_create_text_node(text: *const c_char) -> *mut GpuiElement {
    let text_str = unsafe { cstr_to_str(text) };
    let node = Node::new_text(text_str);
    let mut tree = lock_tree();
    let id = tree.insert(node);
    node_id_to_handle(id)
}

// ---------------------------------------------------------------------------
// 3. appendChild
// ---------------------------------------------------------------------------

/// Append `child` as the last child of `parent`.
#[no_mangle]
pub extern "C" fn gpui_append_child(parent: *mut GpuiElement, child: *mut GpuiElement) {
    let parent_id = unsafe { handle_to_node_id(parent) };
    let child_id = unsafe { handle_to_node_id(child) };
    if parent_id.is_null() || child_id.is_null() {
        return;
    }
    let mut tree = lock_tree();
    tree.append_child(parent_id, child_id);
    window::request_repaint();
}

// ---------------------------------------------------------------------------
// 4. insertBefore
// ---------------------------------------------------------------------------

/// Insert `child` before `reference` within `parent`.
/// If `reference` is null, appends child instead.
#[no_mangle]
pub extern "C" fn gpui_insert_before(
    parent: *mut GpuiElement,
    child: *mut GpuiElement,
    reference: *mut GpuiElement,
) {
    let parent_id = unsafe { handle_to_node_id(parent) };
    let child_id = unsafe { handle_to_node_id(child) };
    let ref_id = unsafe { handle_to_node_id(reference) };
    if parent_id.is_null() || child_id.is_null() {
        return;
    }
    let mut tree = lock_tree();
    tree.insert_before(parent_id, child_id, ref_id);
    window::request_repaint();
}

// ---------------------------------------------------------------------------
// 5. removeChild
// ---------------------------------------------------------------------------

/// Remove `child` from `parent`.
#[no_mangle]
pub extern "C" fn gpui_remove_child(parent: *mut GpuiElement, child: *mut GpuiElement) {
    let parent_id = unsafe { handle_to_node_id(parent) };
    let child_id = unsafe { handle_to_node_id(child) };
    if parent_id.is_null() || child_id.is_null() {
        return;
    }
    let mut tree = lock_tree();
    tree.remove_child(parent_id, child_id);
    window::request_repaint();
}

// ---------------------------------------------------------------------------
// 6. setAttribute
// ---------------------------------------------------------------------------

/// Set attribute `name` to `value` on `node`.
#[no_mangle]
pub extern "C" fn gpui_set_attribute(
    node: *mut GpuiElement,
    name: *const c_char,
    value: *const c_char,
) {
    let node_id = unsafe { handle_to_node_id(node) };
    if node_id.is_null() {
        return;
    }
    let name_str = unsafe { cstr_to_str(name) };
    let value_str = unsafe { cstr_to_str(value) };
    let mut tree = lock_tree();
    if let Some(n) = tree.get_mut(node_id) {
        n.attributes
            .insert(name_str.to_string(), value_str.to_string());
        window::request_repaint();
    }
}

// ---------------------------------------------------------------------------
// 7. removeAttribute
// ---------------------------------------------------------------------------

/// Remove attribute `name` from `node`.
#[no_mangle]
pub extern "C" fn gpui_remove_attribute(node: *mut GpuiElement, name: *const c_char) {
    let node_id = unsafe { handle_to_node_id(node) };
    if node_id.is_null() {
        return;
    }
    let name_str = unsafe { cstr_to_str(name) };
    let mut tree = lock_tree();
    if let Some(n) = tree.get_mut(node_id) {
        n.attributes.remove(name_str);
        window::request_repaint();
    }
}

// ---------------------------------------------------------------------------
// 8. setTextContent
// ---------------------------------------------------------------------------

/// Set the text content of `node`.
#[no_mangle]
pub extern "C" fn gpui_set_text_content(node: *mut GpuiElement, text: *const c_char) {
    let node_id = unsafe { handle_to_node_id(node) };
    if node_id.is_null() {
        return;
    }
    let text_str = unsafe { cstr_to_str(text) };
    let mut tree = lock_tree();
    if let Some(n) = tree.get_mut(node_id) {
        n.set_text_content(text_str);
        window::request_repaint();
    }
}

// ---------------------------------------------------------------------------
// 9. setStyle
// ---------------------------------------------------------------------------

/// Set a style property on `node`.
#[no_mangle]
pub extern "C" fn gpui_set_style(
    node: *mut GpuiElement,
    prop: *const c_char,
    value: *const c_char,
) {
    let node_id = unsafe { handle_to_node_id(node) };
    if node_id.is_null() {
        return;
    }
    let prop_str = unsafe { cstr_to_str(prop) };
    let value_str = unsafe { cstr_to_str(value) };
    let mut tree = lock_tree();
    if let Some(n) = tree.get_mut(node_id) {
        n.styles
            .insert(prop_str.to_string(), value_str.to_string());
        window::request_repaint();
    }
}

// ---------------------------------------------------------------------------
// 10. addEventListener
// ---------------------------------------------------------------------------

/// C function pointer type for event callbacks from Nim.
pub type EventCallback = extern "C" fn();

/// Register a callback for `event` on `node`.
/// The `handler` is a C function pointer that Nim will pass in.
#[no_mangle]
pub extern "C" fn gpui_add_event_listener(
    node: *mut GpuiElement,
    event: *const c_char,
    handler: EventCallback,
) {
    let node_id = unsafe { handle_to_node_id(node) };
    if node_id.is_null() {
        return;
    }
    let event_str = unsafe { cstr_to_str(event) };
    let mut tree = lock_tree();
    if let Some(n) = tree.get_mut(node_id) {
        let listener = EventListener { callback: handler };
        n.event_listeners
            .entry(event_str.to_string())
            .or_default()
            .push(listener);
    }
}

// ---------------------------------------------------------------------------
// 11. firstChild
// ---------------------------------------------------------------------------

/// Return the first child of `node`, or null if it has no children.
#[no_mangle]
pub extern "C" fn gpui_first_child(node: *mut GpuiElement) -> *mut GpuiElement {
    let node_id = unsafe { handle_to_node_id(node) };
    if node_id.is_null() {
        return std::ptr::null_mut();
    }
    let tree = lock_tree();
    let child_id = tree.first_child(node_id);
    node_id_to_handle(child_id)
}

// ---------------------------------------------------------------------------
// 12. nextSibling
// ---------------------------------------------------------------------------

/// Return the next sibling of `node`, or null.
#[no_mangle]
pub extern "C" fn gpui_next_sibling(node: *mut GpuiElement) -> *mut GpuiElement {
    let node_id = unsafe { handle_to_node_id(node) };
    if node_id.is_null() {
        return std::ptr::null_mut();
    }
    let tree = lock_tree();
    let sibling_id = tree.next_sibling(node_id);
    node_id_to_handle(sibling_id)
}

// ---------------------------------------------------------------------------
// 13. parentNode
// ---------------------------------------------------------------------------

/// Return the parent of `node`, or null.
#[no_mangle]
pub extern "C" fn gpui_parent_node(node: *mut GpuiElement) -> *mut GpuiElement {
    let node_id = unsafe { handle_to_node_id(node) };
    if node_id.is_null() {
        return std::ptr::null_mut();
    }
    let tree = lock_tree();
    let parent_id = tree.parent_node(node_id);
    node_id_to_handle(parent_id)
}

// ===========================================================================
// Window / event loop management
// ===========================================================================

/// Launch a GPUI window.
///
/// This creates a root element in the shadow tree and starts the GPUI event loop.
/// The `title` parameter sets the window title.
/// The `width` and `height` parameters set the initial window size.
/// The `root_builder` callback is called with the root element handle so the
/// Nim side can build the initial tree before the event loop starts.
///
/// **Note:** Without the `gpui-backend` feature this is a placeholder that creates
/// the root element and calls the builder callback but does NOT start an actual
/// GPUI window. The actual GPUI integration will be completed in M4+.
pub type RootBuilderCallback = extern "C" fn(root: *mut GpuiElement);

#[no_mangle]
#[allow(unused_variables)]
pub extern "C" fn gpui_launch(
    title: *const c_char,
    width: f64,
    height: f64,
    root_builder: RootBuilderCallback,
) {
    let title_str = unsafe { cstr_to_str(title) };

    // Create a root element in the shadow tree
    let root_node = Node::new_element("root");
    let root_id = {
        let mut tree = lock_tree();
        tree.insert(root_node)
    };

    // Store the root ID globally so the GPUI view can find it
    {
        let mut root = ROOT_NODE_ID.lock().unwrap_or_else(|p| p.into_inner());
        *root = root_id;
    }

    let root_handle = node_id_to_handle(root_id);

    // Call back to Nim so it can build the initial tree
    root_builder(root_handle);

    // When the gpui-backend feature is enabled, launch the actual GPUI
    // window with the shadow tree renderer as the root view.
    #[cfg(feature = "gpui-backend")]
    {
        let win_id = window::create_window(title_str, width, height);
        gpui_app::launch_gpui_app(title_str, width, height, win_id);
    }

    // Without the feature, the function returns after building the shadow tree.
    // This is the existing behavior used for testing and headless operation.
    #[cfg(not(feature = "gpui-backend"))]
    {
        let _ = title_str;
    }
}

/// Trigger all event listeners for the given event on the given node.
/// This is called by the GPUI event loop (M4+) when an event occurs,
/// or can be called directly for testing.
#[no_mangle]
pub extern "C" fn gpui_dispatch_event(node: *mut GpuiElement, event: *const c_char) {
    let node_id = unsafe { handle_to_node_id(node) };
    if node_id.is_null() {
        return;
    }
    let event_str = unsafe { cstr_to_str(event) };

    // Collect callbacks while holding the lock, then call them after releasing.
    // This avoids deadlock when callbacks modify the tree.
    let callbacks: Vec<extern "C" fn()> = {
        let tree = lock_tree();
        if let Some(n) = tree.get(node_id) {
            n.event_listeners
                .get(event_str)
                .map(|listeners| listeners.iter().map(|l| l.callback).collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    };

    for cb in callbacks {
        cb();
    }
}

/// Free a GpuiElement handle.
/// This deallocates the handle pointer but does NOT remove the node from the tree.
/// Call gpui_remove_child first to detach the node, then gpui_destroy_element
/// to free the handle memory.
#[no_mangle]
pub extern "C" fn gpui_destroy_element(handle: *mut GpuiElement) {
    if !handle.is_null() {
        unsafe {
            drop(Box::from_raw(handle));
        }
    }
}

/// Remove a node and all its descendants from the shadow tree entirely.
/// This is for cleanup — it removes the node from the tree store (not just
/// from its parent's children list). The handle is also freed.
#[no_mangle]
pub extern "C" fn gpui_destroy_tree(handle: *mut GpuiElement) {
    if handle.is_null() {
        return;
    }
    let node_id = unsafe { handle_to_node_id(handle) };

    // Collect all descendant IDs via BFS
    let ids_to_remove = {
        let tree = lock_tree();
        let mut to_visit = vec![node_id];
        let mut to_remove = Vec::new();
        while let Some(id) = to_visit.pop() {
            to_remove.push(id);
            if let Some(n) = tree.get(id) {
                to_visit.extend_from_slice(&n.children);
            }
        }
        to_remove
    };

    {
        let mut tree = lock_tree();
        for id in &ids_to_remove {
            tree.remove(*id);
        }
    }

    // Free the handle
    unsafe {
        drop(Box::from_raw(handle));
    }
}

/// Reset the global tree (useful for testing).
#[no_mangle]
pub extern "C" fn gpui_reset_tree() {
    let mut tree = lock_tree();
    *tree = Tree::new();
    let mut root = ROOT_NODE_ID.lock().unwrap_or_else(|p| p.into_inner());
    *root = NodeId::NULL;
}

/// Get the number of nodes in the global tree (useful for debugging/testing).
#[no_mangle]
pub extern "C" fn gpui_tree_node_count() -> u64 {
    let tree = lock_tree();
    tree.len() as u64
}

// ---------------------------------------------------------------------------
// Tree inspection functions (for cross-renderer testing)
// ---------------------------------------------------------------------------

/// Get the number of children of a node.
/// Returns 0 if the node is null or not found.
#[no_mangle]
pub extern "C" fn gpui_child_count(node: *mut GpuiElement) -> u64 {
    let node_id = unsafe { handle_to_node_id(node) };
    if node_id.is_null() {
        return 0;
    }
    let tree = lock_tree();
    tree.get(node_id)
        .map(|n| n.children.len() as u64)
        .unwrap_or(0)
}

/// Get the text content of a node and all its descendants (recursive).
/// For text nodes, returns the text. For element nodes, concatenates
/// all descendant text. The result is written into the provided buffer.
/// Returns the number of bytes needed (excluding null terminator),
/// or 0 if the node is not found.
#[no_mangle]
pub extern "C" fn gpui_get_text_content(
    node: *mut GpuiElement,
    buf: *mut u8,
    buf_len: u64,
) -> u64 {
    let node_id = unsafe { handle_to_node_id(node) };
    if node_id.is_null() {
        return 0;
    }
    let tree = lock_tree();
    let text = tree.collect_text(node_id);
    let bytes = text.as_bytes();
    let needed = bytes.len() as u64;

    if buf.is_null() || buf_len == 0 {
        return needed;
    }

    let to_copy = std::cmp::min(bytes.len(), (buf_len - 1) as usize);
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, to_copy);
        *buf.add(to_copy) = 0; // null terminator
    }
    needed
}

/// Get an attribute value from a node.
/// Returns the number of bytes in the attribute value (excluding null),
/// or 0 if the attribute is not found. Writes into `buf` if provided.
#[no_mangle]
pub extern "C" fn gpui_get_attribute(
    node: *mut GpuiElement,
    name: *const c_char,
    buf: *mut u8,
    buf_len: u64,
) -> u64 {
    let node_id = unsafe { handle_to_node_id(node) };
    if node_id.is_null() {
        return 0;
    }
    let name_str = unsafe { cstr_to_str(name) };
    let tree = lock_tree();
    let value = match tree.get(node_id) {
        Some(n) => match n.attributes.get(name_str) {
            Some(v) => v.clone(),
            None => return 0,
        },
        None => return 0,
    };

    let bytes = value.as_bytes();
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

/// Get the Nth child of a node (0-indexed).
/// Returns null if out of bounds or node not found.
#[no_mangle]
pub extern "C" fn gpui_nth_child(node: *mut GpuiElement, index: u64) -> *mut GpuiElement {
    let node_id = unsafe { handle_to_node_id(node) };
    if node_id.is_null() {
        return std::ptr::null_mut();
    }
    let tree = lock_tree();
    if let Some(n) = tree.get(node_id) {
        if (index as usize) < n.children.len() {
            return node_id_to_handle(n.children[index as usize]);
        }
    }
    std::ptr::null_mut()
}

/// Get the tag name of a node. Returns 0 for text nodes.
/// Writes into `buf` if provided, returns the number of bytes needed.
#[no_mangle]
pub extern "C" fn gpui_get_tag(
    node: *mut GpuiElement,
    buf: *mut u8,
    buf_len: u64,
) -> u64 {
    let node_id = unsafe { handle_to_node_id(node) };
    if node_id.is_null() {
        return 0;
    }
    let tree = lock_tree();
    let tag = match tree.get(node_id) {
        Some(n) => match n.tag() {
            Some(t) => t.to_string(),
            None => return 0,
        },
        None => return 0,
    };

    let bytes = tag.as_bytes();
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

/// Get the GPUI element kind for a node.
/// Returns: 0 = not found, 1 = Div, 2 = TextContainer, 3 = Img, 4 = Svg, 5 = TextNode.
#[no_mangle]
pub extern "C" fn gpui_get_element_kind(node: *mut GpuiElement) -> u8 {
    let node_id = unsafe { handle_to_node_id(node) };
    if node_id.is_null() {
        return 0;
    }
    let tree = lock_tree();
    match tree.get(node_id) {
        Some(n) => match n.gpui_kind {
            tree::GpuiElementKind::Div => 1,
            tree::GpuiElementKind::TextContainer => 2,
            tree::GpuiElementKind::Img => 3,
            tree::GpuiElementKind::Svg => 4,
            tree::GpuiElementKind::TextNode => 5,
        },
        None => 0,
    }
}

// ---------------------------------------------------------------------------
// Window management
// ---------------------------------------------------------------------------

/// Create a new window with the given title and initial size.
/// Returns a window ID (> 0) on success, 0 on failure.
#[no_mangle]
pub extern "C" fn gpui_create_window(
    title: *const c_char,
    width: f64,
    height: f64,
) -> u32 {
    let title_str = unsafe { cstr_to_str(title) };
    window::create_window(title_str, width, height)
}

/// Show a window (transition from Created to Visible state).
///
/// Without the `gpui-backend` feature this just updates the internal state.
/// With the feature enabled, this starts the GPUI event loop for the window.
///
/// Returns 1 on success, 0 if the window was not in Created state or not found.
#[no_mangle]
pub extern "C" fn gpui_show_window(window_id: u32) -> u8 {
    if window::show_window(window_id) {
        1
    } else {
        0
    }
}

/// Request that a window be closed. If an on_close callback is registered
/// and returns 0, the close is denied.
/// Returns 1 if the window was closed, 0 if denied or not found.
#[no_mangle]
pub extern "C" fn gpui_close_window(window_id: u32) -> u8 {
    if window::close_window(window_id) {
        1
    } else {
        0
    }
}

/// Destroy a window and remove it from the registry.
#[no_mangle]
pub extern "C" fn gpui_destroy_window(window_id: u32) {
    window::destroy_window(window_id);
}

/// Get the current state of a window.
/// Returns: 0 = not found, 1 = Created, 2 = Visible, 3 = CloseRequested, 4 = Closed.
#[no_mangle]
pub extern "C" fn gpui_window_state(window_id: u32) -> u8 {
    match window::window_state(window_id) {
        None => 0,
        Some(window::WindowState::Created) => 1,
        Some(window::WindowState::Visible) => 2,
        Some(window::WindowState::CloseRequested) => 3,
        Some(window::WindowState::Closed) => 4,
    }
}

/// Get the current width of a window. Returns 0.0 if not found.
#[no_mangle]
pub extern "C" fn gpui_window_width(window_id: u32) -> f64 {
    window::window_size(window_id)
        .map(|(w, _)| w)
        .unwrap_or(0.0)
}

/// Get the current height of a window. Returns 0.0 if not found.
#[no_mangle]
pub extern "C" fn gpui_window_height(window_id: u32) -> f64 {
    window::window_size(window_id)
        .map(|(_, h)| h)
        .unwrap_or(0.0)
}

/// Request a repaint of the window. This signals that the shadow tree has
/// changed and the window should re-render on the next frame.
#[no_mangle]
pub extern "C" fn gpui_request_repaint() {
    window::request_repaint();
}

/// Check if a repaint has been requested (and clear the flag).
/// Returns 1 if a repaint was pending, 0 otherwise.
#[no_mangle]
pub extern "C" fn gpui_take_repaint_request() -> u8 {
    if window::take_repaint_request() {
        1
    } else {
        0
    }
}

/// Register a callback for window resize events.
/// The callback receives (width: f64, height: f64).
#[no_mangle]
pub extern "C" fn gpui_on_resize(window_id: u32, callback: ResizeCallback) {
    window::with_window_mut(window_id, |w| {
        w.on_resize = Some(callback);
    });
}

/// Register a callback for window focus events.
/// The callback receives (focused: u8) where 1 = focused, 0 = unfocused.
#[no_mangle]
pub extern "C" fn gpui_on_focus(window_id: u32, callback: FocusCallback) {
    window::with_window_mut(window_id, |w| {
        w.on_focus = Some(callback);
    });
}

/// Register a callback for window close requests.
/// The callback should return 1 to allow close, 0 to prevent it.
#[no_mangle]
pub extern "C" fn gpui_on_close(window_id: u32, callback: CloseCallback) {
    window::with_window_mut(window_id, |w| {
        w.on_close = Some(callback);
    });
}

/// Simulate a resize event on a window (for testing / event bridging).
#[no_mangle]
pub extern "C" fn gpui_notify_resize(window_id: u32, width: f64, height: f64) {
    window::notify_resize(window_id, width, height);
}

/// Simulate a focus event on a window (for testing / event bridging).
/// `focused`: 1 = gained focus, 0 = lost focus.
#[no_mangle]
pub extern "C" fn gpui_notify_focus(window_id: u32, focused: u8) {
    window::notify_focus(window_id, focused != 0);
}

/// Reset all windows (for testing).
#[no_mangle]
pub extern "C" fn gpui_reset_windows() {
    window::reset_windows();
}

// ---------------------------------------------------------------------------
// Render plan inspection FFI (G1-G)
// ---------------------------------------------------------------------------

/// Build a render plan from the shadow tree rooted at `root` and return it as
/// a JSON string. The caller must free the returned string with `gpui_free_string`.
///
/// Returns null if the handle is null or the node doesn't exist.
///
/// JSON format:
/// ```json
/// {
///   "kind": "Div",
///   "tag": "div",
///   "text": null,
///   "has_click_handler": false,
///   "has_input_handler": false,
///   "event_names": [],
///   "styles": { "w": "100%", ... },
///   "children": [ ... ]
/// }
/// ```
#[no_mangle]
pub extern "C" fn gpui_render_plan_json(root: *mut GpuiElement) -> *mut c_char {
    let node_id = unsafe { handle_to_node_id(root) };
    if node_id.is_null() {
        return std::ptr::null_mut();
    }
    let tree = lock_tree();
    let plan = match render_sync::build_render_plan(&tree, node_id) {
        Some(p) => p,
        None => return std::ptr::null_mut(),
    };
    drop(tree);

    let json = render_plan_to_json(&plan);
    match std::ffi::CString::new(json) {
        Ok(cs) => cs.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Free a string returned by `gpui_render_plan_json`.
#[no_mangle]
pub extern "C" fn gpui_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            drop(std::ffi::CString::from_raw(ptr));
        }
    }
}

/// Return the total element count in the render plan rooted at `root`.
/// Returns 0 if the handle is null or the node doesn't exist.
#[no_mangle]
pub extern "C" fn gpui_render_plan_element_count(root: *mut GpuiElement) -> u32 {
    let node_id = unsafe { handle_to_node_id(root) };
    if node_id.is_null() {
        return 0;
    }
    let tree = lock_tree();
    match render_sync::build_render_plan(&tree, node_id) {
        Some(plan) => render_sync::count_render_nodes(&plan) as u32,
        None => 0,
    }
}

/// Verify that a render plan can be built from the shadow tree rooted at `root`.
/// Returns 1 if valid, 0 if there are issues (null handle, missing node, etc.).
#[no_mangle]
pub extern "C" fn gpui_verify_render_plan(root: *mut GpuiElement) -> u8 {
    let node_id = unsafe { handle_to_node_id(root) };
    if node_id.is_null() {
        return 0;
    }
    let tree = lock_tree();
    match render_sync::build_render_plan(&tree, node_id) {
        Some(_) => 1,
        None => 0,
    }
}

/// Internal helper: serialize a RenderNode to a JSON string.
fn render_plan_to_json(plan: &render_sync::RenderNode) -> String {
    fn node_to_string(plan: &render_sync::RenderNode) -> String {
        let kind = format!("{:?}", plan.kind);
        let tag = &plan.tag;
        let text = match &plan.text {
            Some(t) => format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\"")),
            None => "null".to_string(),
        };

        let mut style_entries = Vec::new();
        let s = &plan.styles;
        macro_rules! push_style {
            ($field:ident, $name:expr) => {
                if let Some(ref v) = s.$field {
                    style_entries.push(format!("\"{}\":\"{}\"", $name, v.replace('"', "\\\"")));
                }
            };
        }
        push_style!(bg, "bg");
        push_style!(w, "w");
        push_style!(h, "h");
        push_style!(min_w, "min_w");
        push_style!(min_h, "min_h");
        push_style!(max_w, "max_w");
        push_style!(max_h, "max_h");
        push_style!(p, "p");
        push_style!(m, "m");
        push_style!(flex_direction, "flex_direction");
        push_style!(gap, "gap");
        push_style!(text_size, "text_size");
        push_style!(text_color, "text_color");
        push_style!(rounded, "rounded");
        push_style!(items, "items");
        push_style!(justify, "justify");
        push_style!(border_width, "border_width");
        push_style!(border_color, "border_color");
        push_style!(shadow, "shadow");
        push_style!(opacity, "opacity");
        push_style!(overflow, "overflow");
        push_style!(font_family, "font_family");
        push_style!(font_weight, "font_weight");
        push_style!(font_style, "font_style");
        push_style!(line_height, "line_height");
        push_style!(letter_spacing, "letter_spacing");
        push_style!(cursor, "cursor");
        push_style!(display, "display");
        push_style!(position, "position");
        let styles_json = format!("{{{}}}", style_entries.join(","));

        let event_names: Vec<String> = plan
            .event_names
            .iter()
            .map(|e| format!("\"{}\"", e))
            .collect();

        let children: Vec<String> = plan.children.iter().map(node_to_string).collect();

        format!(
            "{{\"kind\":\"{}\",\"tag\":\"{}\",\"text\":{},\"has_click_handler\":{},\"has_input_handler\":{},\"event_names\":[{}],\"styles\":{},\"children\":[{}]}}",
            kind,
            tag.replace('"', "\\\""),
            text,
            plan.has_click_handler,
            plan.has_input_handler,
            event_names.join(","),
            styles_json,
            children.join(","),
        )
    }
    node_to_string(plan)
}

// ===========================================================================
// Rust-side tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::ffi::CString;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Helper to create a CString and return it.
    fn c(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    // -----------------------------------------------------------------------
    // Element creation
    // -----------------------------------------------------------------------

    #[test]
    #[serial]
    fn test_create_element_returns_non_null() {
        gpui_reset_tree();
        let tag = c("div");
        let handle = gpui_create_element(tag.as_ptr());
        assert!(!handle.is_null());
        gpui_destroy_element(handle);
    }

    #[test]
    #[serial]
    fn test_create_text_node_returns_non_null() {
        gpui_reset_tree();
        let text = c("hello");
        let handle = gpui_create_text_node(text.as_ptr());
        assert!(!handle.is_null());
        gpui_destroy_element(handle);
    }

    #[test]
    #[serial]
    fn test_tree_node_count() {
        gpui_reset_tree();
        assert_eq!(gpui_tree_node_count(), 0);

        let tag = c("div");
        let n1 = gpui_create_element(tag.as_ptr());
        assert_eq!(gpui_tree_node_count(), 1);

        let n2 = gpui_create_element(tag.as_ptr());
        assert_eq!(gpui_tree_node_count(), 2);

        gpui_destroy_element(n1);
        gpui_destroy_element(n2);
    }

    // -----------------------------------------------------------------------
    // appendChild, insertBefore, removeChild
    // -----------------------------------------------------------------------

    #[test]
    #[serial]
    fn test_append_and_traverse() {
        gpui_reset_tree();
        let tag_div = c("div");
        let tag_span = c("span");

        let parent = gpui_create_element(tag_div.as_ptr());
        let child1 = gpui_create_element(tag_span.as_ptr());
        let child2 = gpui_create_element(tag_span.as_ptr());

        gpui_append_child(parent, child1);
        gpui_append_child(parent, child2);

        // first_child of parent should be child1
        let fc = gpui_first_child(parent);
        assert!(!fc.is_null());
        assert_eq!(unsafe { (*fc).node_id }, unsafe { (*child1).node_id });
        gpui_destroy_element(fc);

        // next_sibling of child1 should be child2
        let ns = gpui_next_sibling(child1);
        assert!(!ns.is_null());
        assert_eq!(unsafe { (*ns).node_id }, unsafe { (*child2).node_id });
        gpui_destroy_element(ns);

        // parent_node of child1 should be parent
        let pn = gpui_parent_node(child1);
        assert!(!pn.is_null());
        assert_eq!(unsafe { (*pn).node_id }, unsafe { (*parent).node_id });
        gpui_destroy_element(pn);

        gpui_destroy_element(parent);
        gpui_destroy_element(child1);
        gpui_destroy_element(child2);
    }

    #[test]
    #[serial]
    fn test_insert_before() {
        gpui_reset_tree();
        let tag = c("div");

        let parent = gpui_create_element(tag.as_ptr());
        let c1 = gpui_create_element(tag.as_ptr());
        let c2 = gpui_create_element(tag.as_ptr());
        let c3 = gpui_create_element(tag.as_ptr());

        gpui_append_child(parent, c1);
        gpui_append_child(parent, c2);
        gpui_insert_before(parent, c3, c2); // c3 before c2

        // Order should be: c1, c3, c2
        let fc = gpui_first_child(parent);
        assert_eq!(unsafe { (*fc).node_id }, unsafe { (*c1).node_id });

        let ns1 = gpui_next_sibling(c1);
        assert_eq!(unsafe { (*ns1).node_id }, unsafe { (*c3).node_id });

        let ns2 = gpui_next_sibling(c3);
        assert_eq!(unsafe { (*ns2).node_id }, unsafe { (*c2).node_id });

        gpui_destroy_element(fc);
        gpui_destroy_element(ns1);
        gpui_destroy_element(ns2);
        gpui_destroy_element(parent);
        gpui_destroy_element(c1);
        gpui_destroy_element(c2);
        gpui_destroy_element(c3);
    }

    #[test]
    #[serial]
    fn test_insert_before_null_appends() {
        gpui_reset_tree();
        let tag = c("div");

        let parent = gpui_create_element(tag.as_ptr());
        let c1 = gpui_create_element(tag.as_ptr());
        let c2 = gpui_create_element(tag.as_ptr());

        gpui_append_child(parent, c1);
        gpui_insert_before(parent, c2, std::ptr::null_mut());

        assert_eq!(gpui_child_count(parent), 2);
        let nth0 = gpui_nth_child(parent, 0);
        let nth1 = gpui_nth_child(parent, 1);
        assert_eq!(unsafe { (*nth0).node_id }, unsafe { (*c1).node_id });
        assert_eq!(unsafe { (*nth1).node_id }, unsafe { (*c2).node_id });

        gpui_destroy_element(nth0);
        gpui_destroy_element(nth1);
        gpui_destroy_element(parent);
        gpui_destroy_element(c1);
        gpui_destroy_element(c2);
    }

    #[test]
    #[serial]
    fn test_remove_child() {
        gpui_reset_tree();
        let tag = c("div");

        let parent = gpui_create_element(tag.as_ptr());
        let child = gpui_create_element(tag.as_ptr());

        gpui_append_child(parent, child);
        gpui_remove_child(parent, child);

        let fc = gpui_first_child(parent);
        assert!(fc.is_null());

        let pn = gpui_parent_node(child);
        assert!(pn.is_null());

        gpui_destroy_element(parent);
        gpui_destroy_element(child);
    }

    #[test]
    #[serial]
    fn test_reparent_via_append() {
        gpui_reset_tree();
        let tag = c("div");

        let p1 = gpui_create_element(tag.as_ptr());
        let p2 = gpui_create_element(tag.as_ptr());
        let child = gpui_create_element(tag.as_ptr());

        gpui_append_child(p1, child);
        assert_eq!(gpui_child_count(p1), 1);

        // Reparent to p2 (should auto-detach from p1)
        gpui_append_child(p2, child);
        assert_eq!(gpui_child_count(p1), 0);
        assert_eq!(gpui_child_count(p2), 1);

        let pn = gpui_parent_node(child);
        assert_eq!(unsafe { (*pn).node_id }, unsafe { (*p2).node_id });

        gpui_destroy_element(pn);
        gpui_destroy_element(p1);
        gpui_destroy_element(p2);
        gpui_destroy_element(child);
    }

    // -----------------------------------------------------------------------
    // Attributes
    // -----------------------------------------------------------------------

    #[test]
    #[serial]
    fn test_set_and_remove_attribute() {
        gpui_reset_tree();
        let tag = c("div");
        let name = c("width");
        let value = c("100%");

        let node = gpui_create_element(tag.as_ptr());
        gpui_set_attribute(node, name.as_ptr(), value.as_ptr());

        // Read it back
        let mut buf = vec![0u8; 64];
        let len = gpui_get_attribute(node, name.as_ptr(), buf.as_mut_ptr(), 64);
        assert_eq!(len, 4); // "100%"
        let result = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) };
        assert_eq!(result.to_str().unwrap(), "100%");

        gpui_remove_attribute(node, name.as_ptr());

        let len = gpui_get_attribute(node, name.as_ptr(), buf.as_mut_ptr(), 64);
        assert_eq!(len, 0);

        gpui_destroy_element(node);
    }

    #[test]
    #[serial]
    fn test_attribute_overwrite() {
        gpui_reset_tree();
        let tag = c("div");
        let name = c("class");
        let val1 = c("old");
        let val2 = c("new");

        let node = gpui_create_element(tag.as_ptr());
        gpui_set_attribute(node, name.as_ptr(), val1.as_ptr());
        gpui_set_attribute(node, name.as_ptr(), val2.as_ptr());

        let mut buf = vec![0u8; 64];
        let len = gpui_get_attribute(node, name.as_ptr(), buf.as_mut_ptr(), 64);
        assert_eq!(len, 3);
        let result = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) };
        assert_eq!(result.to_str().unwrap(), "new");

        gpui_destroy_element(node);
    }

    // -----------------------------------------------------------------------
    // Text content
    // -----------------------------------------------------------------------

    #[test]
    #[serial]
    fn test_set_text_content() {
        gpui_reset_tree();
        let text1 = c("hello");
        let text2 = c("world");

        let node = gpui_create_text_node(text1.as_ptr());
        gpui_set_text_content(node, text2.as_ptr());

        let mut buf = vec![0u8; 64];
        let len = gpui_get_text_content(node, buf.as_mut_ptr(), 64);
        assert_eq!(len, 5);
        let result = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) };
        assert_eq!(result.to_str().unwrap(), "world");

        gpui_destroy_element(node);
    }

    #[test]
    #[serial]
    fn test_get_text_content_recursive() {
        gpui_reset_tree();
        let tag = c("div");
        let parent = gpui_create_element(tag.as_ptr());
        let t1 = c("hello ");
        let t2 = c("world");
        let child1 = gpui_create_text_node(t1.as_ptr());
        let child2 = gpui_create_text_node(t2.as_ptr());
        gpui_append_child(parent, child1);
        gpui_append_child(parent, child2);

        let mut buf = vec![0u8; 64];
        let len = gpui_get_text_content(parent, buf.as_mut_ptr(), 64);
        assert_eq!(len, 11);
        let result = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) };
        assert_eq!(result.to_str().unwrap(), "hello world");

        gpui_destroy_element(parent);
        gpui_destroy_element(child1);
        gpui_destroy_element(child2);
    }

    #[test]
    #[serial]
    fn test_get_text_content_query_size() {
        gpui_reset_tree();
        let text = c("hello world");
        let node = gpui_create_text_node(text.as_ptr());

        // Query size without buffer
        let needed = gpui_get_text_content(node, std::ptr::null_mut(), 0);
        assert_eq!(needed, 11);

        gpui_destroy_element(node);
    }

    // -----------------------------------------------------------------------
    // Styles
    // -----------------------------------------------------------------------

    #[test]
    #[serial]
    fn test_set_style() {
        gpui_reset_tree();
        let tag = c("div");
        let prop = c("background");
        let value = c("rgb(255, 0, 0)");

        let node = gpui_create_element(tag.as_ptr());
        gpui_set_style(node, prop.as_ptr(), value.as_ptr());

        {
            let tree = lock_tree();
            let nid = NodeId(unsafe { (*node).node_id });
            let n = tree.get(nid).unwrap();
            assert_eq!(
                n.styles.get("background").map(|s| s.as_str()),
                Some("rgb(255, 0, 0)")
            );
        }

        gpui_destroy_element(node);
    }

    #[test]
    #[serial]
    fn test_style_overwrite() {
        gpui_reset_tree();
        let tag = c("div");
        let prop = c("width");
        let val1 = c("100px");
        let val2 = c("200px");

        let node = gpui_create_element(tag.as_ptr());
        gpui_set_style(node, prop.as_ptr(), val1.as_ptr());
        gpui_set_style(node, prop.as_ptr(), val2.as_ptr());

        {
            let tree = lock_tree();
            let nid = NodeId(unsafe { (*node).node_id });
            let n = tree.get(nid).unwrap();
            assert_eq!(n.styles.get("width").map(|s| s.as_str()), Some("200px"));
        }

        gpui_destroy_element(node);
    }

    // -----------------------------------------------------------------------
    // Events
    // -----------------------------------------------------------------------

    #[test]
    #[serial]
    fn test_add_event_listener_and_dispatch() {
        gpui_reset_tree();
        let tag = c("div");
        let event = c("click");

        static CALL_COUNT: AtomicU32 = AtomicU32::new(0);

        extern "C" fn test_handler() {
            CALL_COUNT.fetch_add(1, Ordering::SeqCst);
        }

        CALL_COUNT.store(0, Ordering::SeqCst);

        let node = gpui_create_element(tag.as_ptr());
        gpui_add_event_listener(node, event.as_ptr(), test_handler);

        assert_eq!(CALL_COUNT.load(Ordering::SeqCst), 0);

        gpui_dispatch_event(node, event.as_ptr());
        assert_eq!(CALL_COUNT.load(Ordering::SeqCst), 1);

        gpui_dispatch_event(node, event.as_ptr());
        assert_eq!(CALL_COUNT.load(Ordering::SeqCst), 2);

        gpui_destroy_element(node);
    }

    #[test]
    #[serial]
    fn test_dispatch_nonexistent_event() {
        gpui_reset_tree();
        let tag = c("div");
        let click = c("click");
        let hover = c("hover");

        static CALL_COUNT: AtomicU32 = AtomicU32::new(0);

        extern "C" fn handler() {
            CALL_COUNT.fetch_add(1, Ordering::SeqCst);
        }

        CALL_COUNT.store(0, Ordering::SeqCst);

        let node = gpui_create_element(tag.as_ptr());
        gpui_add_event_listener(node, click.as_ptr(), handler);

        // Dispatch a different event
        gpui_dispatch_event(node, hover.as_ptr());
        assert_eq!(CALL_COUNT.load(Ordering::SeqCst), 0);

        gpui_destroy_element(node);
    }

    #[test]
    #[serial]
    fn test_multiple_event_listeners() {
        gpui_reset_tree();
        let tag = c("div");
        let event = c("click");

        static COUNT_A: AtomicU32 = AtomicU32::new(0);
        static COUNT_B: AtomicU32 = AtomicU32::new(0);

        extern "C" fn handler_a() {
            COUNT_A.fetch_add(1, Ordering::SeqCst);
        }
        extern "C" fn handler_b() {
            COUNT_B.fetch_add(1, Ordering::SeqCst);
        }

        COUNT_A.store(0, Ordering::SeqCst);
        COUNT_B.store(0, Ordering::SeqCst);

        let node = gpui_create_element(tag.as_ptr());
        gpui_add_event_listener(node, event.as_ptr(), handler_a);
        gpui_add_event_listener(node, event.as_ptr(), handler_b);

        gpui_dispatch_event(node, event.as_ptr());
        assert_eq!(COUNT_A.load(Ordering::SeqCst), 1);
        assert_eq!(COUNT_B.load(Ordering::SeqCst), 1);

        gpui_destroy_element(node);
    }

    // -----------------------------------------------------------------------
    // Tree inspection
    // -----------------------------------------------------------------------

    #[test]
    #[serial]
    fn test_child_count() {
        gpui_reset_tree();
        let tag = c("div");
        let parent = gpui_create_element(tag.as_ptr());
        assert_eq!(gpui_child_count(parent), 0);

        let c1 = gpui_create_element(tag.as_ptr());
        let c2 = gpui_create_element(tag.as_ptr());
        gpui_append_child(parent, c1);
        assert_eq!(gpui_child_count(parent), 1);
        gpui_append_child(parent, c2);
        assert_eq!(gpui_child_count(parent), 2);

        gpui_remove_child(parent, c1);
        assert_eq!(gpui_child_count(parent), 1);

        gpui_destroy_element(parent);
        gpui_destroy_element(c1);
        gpui_destroy_element(c2);
    }

    #[test]
    #[serial]
    fn test_nth_child() {
        gpui_reset_tree();
        let tag = c("div");
        let parent = gpui_create_element(tag.as_ptr());
        let c1 = gpui_create_element(tag.as_ptr());
        let c2 = gpui_create_element(tag.as_ptr());
        gpui_append_child(parent, c1);
        gpui_append_child(parent, c2);

        let nth0 = gpui_nth_child(parent, 0);
        assert!(!nth0.is_null());
        assert_eq!(unsafe { (*nth0).node_id }, unsafe { (*c1).node_id });

        let nth1 = gpui_nth_child(parent, 1);
        assert!(!nth1.is_null());
        assert_eq!(unsafe { (*nth1).node_id }, unsafe { (*c2).node_id });

        let nth2 = gpui_nth_child(parent, 2);
        assert!(nth2.is_null());

        gpui_destroy_element(nth0);
        gpui_destroy_element(nth1);
        gpui_destroy_element(parent);
        gpui_destroy_element(c1);
        gpui_destroy_element(c2);
    }

    #[test]
    #[serial]
    fn test_get_tag() {
        gpui_reset_tree();
        let tag = c("button");
        let node = gpui_create_element(tag.as_ptr());

        let mut buf = vec![0u8; 64];
        let len = gpui_get_tag(node, buf.as_mut_ptr(), 64);
        assert_eq!(len, 6); // "button"
        let result = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) };
        assert_eq!(result.to_str().unwrap(), "button");

        gpui_destroy_element(node);
    }

    #[test]
    #[serial]
    fn test_get_tag_text_node_returns_zero() {
        gpui_reset_tree();
        let text = c("hello");
        let node = gpui_create_text_node(text.as_ptr());

        let len = gpui_get_tag(node, std::ptr::null_mut(), 0);
        assert_eq!(len, 0);

        gpui_destroy_element(node);
    }

    #[test]
    #[serial]
    fn test_get_element_kind() {
        gpui_reset_tree();

        let div = gpui_create_element(c("div").as_ptr());
        assert_eq!(gpui_get_element_kind(div), 1); // Div

        let span = gpui_create_element(c("span").as_ptr());
        assert_eq!(gpui_get_element_kind(span), 2); // TextContainer

        let img = gpui_create_element(c("img").as_ptr());
        assert_eq!(gpui_get_element_kind(img), 3); // Img

        let svg = gpui_create_element(c("svg").as_ptr());
        assert_eq!(gpui_get_element_kind(svg), 4); // Svg

        let text = gpui_create_text_node(c("hello").as_ptr());
        assert_eq!(gpui_get_element_kind(text), 5); // TextNode

        assert_eq!(gpui_get_element_kind(std::ptr::null_mut()), 0); // Not found

        gpui_destroy_element(div);
        gpui_destroy_element(span);
        gpui_destroy_element(img);
        gpui_destroy_element(svg);
        gpui_destroy_element(text);
    }

    // -----------------------------------------------------------------------
    // Null safety
    // -----------------------------------------------------------------------

    #[test]
    #[serial]
    fn test_null_safety() {
        let tag = c("div");
        let name = c("width");
        let value = c("100");

        gpui_append_child(std::ptr::null_mut(), std::ptr::null_mut());
        gpui_insert_before(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
        gpui_remove_child(std::ptr::null_mut(), std::ptr::null_mut());
        gpui_set_attribute(std::ptr::null_mut(), name.as_ptr(), value.as_ptr());
        gpui_remove_attribute(std::ptr::null_mut(), name.as_ptr());
        gpui_set_text_content(std::ptr::null_mut(), tag.as_ptr());
        gpui_set_style(std::ptr::null_mut(), name.as_ptr(), value.as_ptr());

        extern "C" fn noop() {}
        gpui_add_event_listener(std::ptr::null_mut(), name.as_ptr(), noop);

        let fc = gpui_first_child(std::ptr::null_mut());
        assert!(fc.is_null());
        let ns = gpui_next_sibling(std::ptr::null_mut());
        assert!(ns.is_null());
        let pn = gpui_parent_node(std::ptr::null_mut());
        assert!(pn.is_null());

        assert_eq!(gpui_child_count(std::ptr::null_mut()), 0);
        assert_eq!(gpui_get_text_content(std::ptr::null_mut(), std::ptr::null_mut(), 0), 0);
        assert_eq!(gpui_get_attribute(std::ptr::null_mut(), name.as_ptr(), std::ptr::null_mut(), 0), 0);
        assert_eq!(gpui_get_tag(std::ptr::null_mut(), std::ptr::null_mut(), 0), 0);
        assert_eq!(gpui_get_element_kind(std::ptr::null_mut()), 0);
        let nth = gpui_nth_child(std::ptr::null_mut(), 0);
        assert!(nth.is_null());

        gpui_destroy_element(std::ptr::null_mut());
        gpui_destroy_tree(std::ptr::null_mut());

        gpui_dispatch_event(std::ptr::null_mut(), name.as_ptr());
    }

    // -----------------------------------------------------------------------
    // Destroy tree
    // -----------------------------------------------------------------------

    #[test]
    #[serial]
    fn test_destroy_tree() {
        gpui_reset_tree();
        let tag = c("div");

        let root = gpui_create_element(tag.as_ptr());
        let c1 = gpui_create_element(tag.as_ptr());
        let c2 = gpui_create_element(tag.as_ptr());
        let gc = gpui_create_element(tag.as_ptr());

        gpui_append_child(root, c1);
        gpui_append_child(root, c2);
        gpui_append_child(c1, gc);

        assert_eq!(gpui_tree_node_count(), 4);

        // destroy_tree on root should remove all 4 nodes
        gpui_destroy_tree(root);
        assert_eq!(gpui_tree_node_count(), 0);

        // The handles c1, c2, gc are now dangling but we should NOT double-free root
        // (it was already freed by destroy_tree). We can still free the others
        // since destroy_element only frees the Box, not the tree node.
        gpui_destroy_element(c1);
        gpui_destroy_element(c2);
        gpui_destroy_element(gc);
    }

    // -----------------------------------------------------------------------
    // Window management
    // -----------------------------------------------------------------------

    #[test]
    #[serial]
    fn test_create_window() {
        gpui_reset_windows();
        let title = c("My Window");
        let id = gpui_create_window(title.as_ptr(), 800.0, 600.0);
        assert!(id > 0);
        assert_eq!(gpui_window_state(id), 1); // Created
        assert_eq!(gpui_window_width(id), 800.0);
        assert_eq!(gpui_window_height(id), 600.0);
        gpui_destroy_window(id);
    }

    #[test]
    #[serial]
    fn test_show_and_close_window() {
        gpui_reset_windows();
        let title = c("Test Window");
        let id = gpui_create_window(title.as_ptr(), 640.0, 480.0);
        assert_eq!(gpui_show_window(id), 1);
        assert_eq!(gpui_window_state(id), 2); // Visible
        // Cannot show again
        assert_eq!(gpui_show_window(id), 0);
        // Close
        assert_eq!(gpui_close_window(id), 1);
        assert_eq!(gpui_window_state(id), 4); // Closed
        gpui_destroy_window(id);
    }

    #[test]
    #[serial]
    fn test_window_lifecycle_callbacks() {
        gpui_reset_windows();
        let title = c("Callback Test");
        let id = gpui_create_window(title.as_ptr(), 800.0, 600.0);

        static RESIZE_CALLED: AtomicU32 = AtomicU32::new(0);
        static FOCUS_CALLED: AtomicU32 = AtomicU32::new(0);

        extern "C" fn on_resize(_w: f64, _h: f64) {
            RESIZE_CALLED.fetch_add(1, Ordering::SeqCst);
        }
        extern "C" fn on_focus(_f: u8) {
            FOCUS_CALLED.fetch_add(1, Ordering::SeqCst);
        }
        extern "C" fn deny_close() -> u8 {
            0
        }

        RESIZE_CALLED.store(0, Ordering::SeqCst);
        FOCUS_CALLED.store(0, Ordering::SeqCst);

        gpui_on_resize(id, on_resize);
        gpui_on_focus(id, on_focus);
        gpui_on_close(id, deny_close);

        gpui_show_window(id);

        // Trigger events
        gpui_notify_resize(id, 1024.0, 768.0);
        assert_eq!(RESIZE_CALLED.load(Ordering::SeqCst), 1);
        assert_eq!(gpui_window_width(id), 1024.0);
        assert_eq!(gpui_window_height(id), 768.0);

        gpui_notify_focus(id, 1);
        assert_eq!(FOCUS_CALLED.load(Ordering::SeqCst), 1);

        // Close should be denied
        assert_eq!(gpui_close_window(id), 0);
        assert_eq!(gpui_window_state(id), 2); // Still Visible

        gpui_destroy_window(id);
    }

    #[test]
    #[serial]
    fn test_repaint_on_tree_mutation() {
        gpui_reset_tree();
        gpui_reset_windows();
        // Clear any pending repaint
        gpui_take_repaint_request();

        let tag = c("div");
        let parent = gpui_create_element(tag.as_ptr());
        let child = gpui_create_element(tag.as_ptr());
        gpui_take_repaint_request(); // clear

        gpui_append_child(parent, child);
        assert_eq!(gpui_take_repaint_request(), 1);

        let name = c("width");
        let value = c("100");
        gpui_set_attribute(parent, name.as_ptr(), value.as_ptr());
        assert_eq!(gpui_take_repaint_request(), 1);

        let prop = c("background");
        let val = c("red");
        gpui_set_style(parent, prop.as_ptr(), val.as_ptr());
        assert_eq!(gpui_take_repaint_request(), 1);

        let text = c("hello");
        gpui_set_text_content(parent, text.as_ptr());
        assert_eq!(gpui_take_repaint_request(), 1);

        gpui_remove_child(parent, child);
        assert_eq!(gpui_take_repaint_request(), 1);

        // No more pending
        assert_eq!(gpui_take_repaint_request(), 0);

        gpui_destroy_element(parent);
        gpui_destroy_element(child);
    }

    #[test]
    #[serial]
    fn test_window_not_found() {
        gpui_reset_windows();
        assert_eq!(gpui_window_state(999), 0);
        assert_eq!(gpui_window_width(999), 0.0);
        assert_eq!(gpui_window_height(999), 0.0);
        assert_eq!(gpui_show_window(999), 0);
        assert_eq!(gpui_close_window(999), 0);
        // These should not crash
        gpui_destroy_window(999);
        gpui_on_resize(999, {
            extern "C" fn noop(_: f64, _: f64) {}
            noop
        });
        gpui_on_focus(999, {
            extern "C" fn noop(_: u8) {}
            noop
        });
        gpui_on_close(999, {
            extern "C" fn noop() -> u8 {
                1
            }
            noop
        });
        gpui_notify_resize(999, 100.0, 100.0);
        gpui_notify_focus(999, 1);
    }

    #[test]
    #[serial]
    fn test_launch_callback() {
        gpui_reset_tree();

        static BUILDER_CALLED: AtomicU32 = AtomicU32::new(0);

        extern "C" fn test_builder(root: *mut GpuiElement) {
            assert!(!root.is_null());
            BUILDER_CALLED.fetch_add(1, Ordering::SeqCst);

            // Build a small tree inside the callback
            let tag = CString::new("span").unwrap();
            let child = gpui_create_element(tag.as_ptr());
            gpui_append_child(root, child);
            gpui_destroy_element(child);
        }

        BUILDER_CALLED.store(0, Ordering::SeqCst);

        let title = c("Test Window");
        gpui_launch(title.as_ptr(), 800.0, 600.0, test_builder);

        assert_eq!(BUILDER_CALLED.load(Ordering::SeqCst), 1);
        // Root + span child = 2 nodes
        assert_eq!(gpui_tree_node_count(), 2);
    }

    // -----------------------------------------------------------------------
    // Tag-to-GPUI-kind mapping (via FFI)
    // -----------------------------------------------------------------------

    #[test]
    #[serial]
    fn test_element_kind_mapping_via_ffi() {
        gpui_reset_tree();

        // Container tags -> Div (1)
        for tag_name in &["div", "section", "article", "nav", "header", "footer", "button", "ul", "li"] {
            let tag = c(tag_name);
            let node = gpui_create_element(tag.as_ptr());
            assert_eq!(gpui_get_element_kind(node), 1, "Expected Div for tag '{}'", tag_name);
            gpui_destroy_element(node);
        }

        // Text tags -> TextContainer (2)
        for tag_name in &["span", "p", "h1", "h2", "h3", "label", "strong", "em"] {
            let tag = c(tag_name);
            let node = gpui_create_element(tag.as_ptr());
            assert_eq!(gpui_get_element_kind(node), 2, "Expected TextContainer for tag '{}'", tag_name);
            gpui_destroy_element(node);
        }

        // Img -> 3
        let img = gpui_create_element(c("img").as_ptr());
        assert_eq!(gpui_get_element_kind(img), 3);
        gpui_destroy_element(img);

        // Svg -> 4
        let svg = gpui_create_element(c("svg").as_ptr());
        assert_eq!(gpui_get_element_kind(svg), 4);
        gpui_destroy_element(svg);
    }

    // -----------------------------------------------------------------------
    // Repaint flag
    // -----------------------------------------------------------------------

    #[test]
    #[serial]
    fn test_repaint_request_explicit() {
        gpui_reset_windows();
        gpui_take_repaint_request(); // clear

        assert_eq!(gpui_take_repaint_request(), 0);
        gpui_request_repaint();
        assert_eq!(gpui_take_repaint_request(), 1);
        assert_eq!(gpui_take_repaint_request(), 0); // cleared
    }
}
