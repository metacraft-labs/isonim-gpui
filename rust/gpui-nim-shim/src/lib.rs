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

#[no_mangle]
pub extern "C" fn gpui_create_element(tag: *const c_char) -> *mut GpuiElement {
    let tag = c_str_to_string(tag);
    let mut tree = lock_tree();
    let id = tree.add_node(tag);
    box_handle(id)
}

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

#[no_mangle]
pub extern "C" fn gpui_remove_attribute(node: *mut GpuiElement, name: *const c_char) {
    let Some(nid) = unbox_id(node) else { return };
    let name = c_str_to_string(name);
    let mut tree = lock_tree();
    if let Some(n) = tree.find_mut(nid) {
        n.attributes.retain(|(k, _)| k != &name);
    }
}

#[no_mangle]
pub extern "C" fn gpui_set_text_content(node: *mut GpuiElement, text: *const c_char) {
    let Some(nid) = unbox_id(node) else { return };
    let text = c_str_to_string(text);
    let mut tree = lock_tree();
    if let Some(n) = tree.find_mut(nid) {
        n.text = text;
    }
}

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
