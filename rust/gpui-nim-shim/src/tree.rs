//! Shadow element tree for imperative DOM-like manipulation.
//!
//! GPUI is declarative — UI is expressed by `Render::render()` returning an element
//! tree each frame. IsoNim's RendererBackend needs imperative tree manipulation
//! (createElement, appendChild, etc.).
//!
//! This module bridges the gap by maintaining a mutable tree of `Node` structs.
//! Each node has a tag, attributes, styles, text content, children, parent,
//! and event listeners. The extern "C" shim functions manipulate this tree,
//! and a separate render-sync step (M4+) will translate it into GPUI's
//! declarative element builders.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Globally unique node identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub u64);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

impl NodeId {
    pub fn new() -> Self {
        NodeId(NEXT_ID.fetch_add(1, Ordering::Relaxed))
    }

    /// The null/sentinel id (0), representing "no node".
    pub const NULL: NodeId = NodeId(0);

    pub fn is_null(self) -> bool {
        self.0 == 0
    }
}

/// The kind of a node in the shadow tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    /// An element node (e.g. "div", "span", "img").
    Element(String),
    /// A text node with content.
    Text(String),
}

/// Which GPUI element type this shadow node maps to during render-sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuiElementKind {
    /// Maps to `gpui::div()` — general-purpose flexbox container.
    Div,
    /// Maps to `gpui::div()` with text child — text-oriented elements.
    TextContainer,
    /// Maps to `gpui::img()` — image element.
    Img,
    /// Maps to `gpui::svg()` — SVG element.
    Svg,
    /// Maps to a string child of the parent div — text node.
    TextNode,
}

/// Determine the GPUI element kind for a given HTML tag.
pub fn tag_to_gpui_kind(tag: &str) -> GpuiElementKind {
    match tag {
        // Text node
        "#text" => GpuiElementKind::TextNode,

        // Image element
        "img" => GpuiElementKind::Img,

        // SVG element
        "svg" => GpuiElementKind::Svg,

        // Text-oriented elements (rendered as div + text child with appropriate styling)
        "span" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "label" | "strong" | "em"
        | "code" | "pre" | "a" => GpuiElementKind::TextContainer,

        // Everything else is a div (container elements, form elements, etc.)
        // "div", "section", "article", "nav", "header", "footer",
        // "button", "input", "textarea", "ul", "ol", "li",
        // "form", "fieldset", "main", "aside", "root", ...
        _ => GpuiElementKind::Div,
    }
}

/// A single node in the shadow tree.
#[derive(Debug)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    /// What GPUI element this maps to during render-sync.
    pub gpui_kind: GpuiElementKind,
    /// Attributes set via setAttribute (name -> value).
    pub attributes: HashMap<String, String>,
    /// Style properties set via setStyle (property -> value).
    pub styles: HashMap<String, String>,
    /// Ordered list of child node IDs.
    pub children: Vec<NodeId>,
    /// Parent node ID (NULL if this is a root or detached).
    pub parent: NodeId,
    /// Event listeners keyed by event name.
    /// Each event can have multiple listeners.
    pub event_listeners: HashMap<String, Vec<EventListener>>,

    // -----------------------------------------------------------------
    // PLAT-38: element focus, and the delivery record
    // -----------------------------------------------------------------
    /// Whether this element can take focus. Declared by the caller via
    /// `gpui_set_focusable`; the focus ORDER is the pre-order walk of the
    /// elements carrying it.
    pub focusable: bool,

    /// **Whether this element currently holds focus — stored PER NODE, and
    /// that is a deliberate design choice with a cost.**
    ///
    /// A single `focused: NodeId` on `Tree` would be cheaper and would make
    /// exclusivity true by construction. It would also make PLAT-38's focus
    /// partition law — *"the number of elements holding focus is exactly
    /// zero or one, asserted over the whole element tree"* — unfalsifiable:
    /// there would be no state in which two elements hold it, so the
    /// published killing mutation (*"let two elements hold it"*) could not
    /// be performed and the law would be a sentence rather than a check.
    /// `codetracer-specs/Testing/Verification-Harness-Traps.md` §36 is
    /// exactly this shape — a published killer that cannot kill its own law
    /// because the implementation chose an invariant that swallows it — and
    /// PLAT-38's own text says to read the killer against the
    /// implementation before trusting the law.
    ///
    /// So exclusivity is ESTABLISHED by `Tree::focus_only`, which clears
    /// every other flag, and OBSERVED by `Tree::focused_ids`, which counts
    /// them. The cost is that focusing is O(n) in the tree rather than
    /// O(1); measured against the alternative, an unkillable law is the
    /// more expensive of the two.
    pub focused: bool,

    /// Whether this element traps focus inside its own subtree. This is what
    /// makes the vocabulary's `Modal` expressible: the entry's specified
    /// behaviour is *a region that takes exclusive input until dismissed*,
    /// and a trap is that statement in the renderer rather than in prose.
    pub focus_trap: bool,

    /// **The last event DELIVERED to this element, recorded by the shim
    /// before any callback runs.** This is the Rust-side element store that
    /// PLAT-38's gate reads: a Nim handler cannot write it without crossing
    /// the FFI boundary, so a binding that applied a key on its own side —
    /// or an adapter that emulated delivery over the shadow tree — leaves it
    /// untouched.
    pub last_event: Option<DeliveredEvent>,

    /// How many LISTENERS the deliveries to this element actually reached.
    /// `last_event.is_some()` says a key arrived; this says whether it
    /// reached anything, which is the difference PLAT-38's gate names
    /// between *"a key that reaches nothing"* and *"a key that reaches
    /// everything"*.
    pub delivery_count: u64,
}

/// One event, as the element store recorded it arriving.
///
/// The key is kept as a BASE NAME plus a MODIFIER BITMASK rather than as a
/// rendered string such as `"Shift+F10"`. The reason is
/// `Verification-Harness-Traps.md` §25: a helper that silently drops a
/// modifier it cannot spell hands you a test about a different key, and that
/// has already been paid for once in this campaign. A bitmask cannot be
/// dropped without the number changing, and the number is what the gate
/// asserts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveredEvent {
    /// The event name it arrived under (`"keydown"`, `"click"`, …).
    pub event: String,
    /// One of the `GPUI_EVENT_*` kinds.
    pub kind: u32,
    /// The base key name, unmodified — `"Up"`, `"F10"`, `"a"`, `"Space"`,
    /// `"é"`. Empty for an event that carries no key.
    pub key: String,
    /// `GPUI_MOD_*` bits.
    pub modifiers: u32,
    /// Whether the platform reported this as an auto-repeat.
    pub repeat: bool,
    /// Process-wide monotonic delivery sequence number, starting at 1.
    /// A node that has never received anything answers 0, which is how
    /// "nothing arrived" is distinguished from "an identical thing arrived
    /// again".
    pub seq: u64,
}

/// An event listener stored in the shadow tree.
///
/// Supports two dispatch modes:
/// - **Legacy (function pointer):** `callback` is set, `callback_id` is 0.
///   The function pointer is called directly.
/// - **Dispatcher (callback ID):** `callback_id` > 0, dispatched via the
///   global event dispatcher registered by `gpui_set_event_dispatcher`.
///   `callback` is set to a dummy no-op in this mode.
#[derive(Debug, Clone, Copy)]
pub struct EventListener {
    /// PLAT-38 widened this from `extern "C" fn()`. See `crate::EventCallback`.
    pub callback: crate::EventCallback,
    pub callback_id: i32,
}

impl Node {
    /// Create a new element node with the given tag.
    pub fn new_element(tag: &str) -> Self {
        let gpui_kind = tag_to_gpui_kind(tag);
        Node {
            id: NodeId::new(),
            kind: NodeKind::Element(tag.to_string()),
            gpui_kind,
            attributes: HashMap::new(),
            styles: HashMap::new(),
            children: Vec::new(),
            parent: NodeId::NULL,
            event_listeners: HashMap::new(),
            focusable: false,
            focused: false,
            focus_trap: false,
            last_event: None,
            delivery_count: 0,
        }
    }

    /// Create a new text node with the given content.
    pub fn new_text(text: &str) -> Self {
        Node {
            id: NodeId::new(),
            kind: NodeKind::Text(text.to_string()),
            gpui_kind: GpuiElementKind::TextNode,
            attributes: HashMap::new(),
            styles: HashMap::new(),
            children: Vec::new(),
            parent: NodeId::NULL,
            event_listeners: HashMap::new(),
            focusable: false,
            focused: false,
            focus_trap: false,
            last_event: None,
            delivery_count: 0,
        }
    }

    /// Get the tag name, if this is an element node.
    pub fn tag(&self) -> Option<&str> {
        match &self.kind {
            NodeKind::Element(tag) => Some(tag.as_str()),
            NodeKind::Text(_) => None,
        }
    }

    /// Get the text content.
    pub fn text_content(&self) -> Option<&str> {
        match &self.kind {
            NodeKind::Text(text) => Some(text.as_str()),
            NodeKind::Element(_) => None,
        }
    }

    /// Set the text content (for text nodes, replaces text; for element nodes,
    /// stores as __text_content attribute for render-sync).
    pub fn set_text_content(&mut self, text: &str) {
        match &mut self.kind {
            NodeKind::Text(ref mut t) => *t = text.to_string(),
            NodeKind::Element(_) => {
                // For element nodes, setting text content could clear children
                // and set inner text. We store it as a special attribute.
                // The render-sync will use this to add a text child.
                self.attributes
                    .insert("__text_content".to_string(), text.to_string());
            }
        }
    }
}

/// The shadow tree: a flat store of nodes indexed by NodeId.
#[derive(Debug)]
pub struct Tree {
    nodes: HashMap<u64, Node>,
}

impl Tree {
    pub fn new() -> Self {
        Tree {
            nodes: HashMap::new(),
        }
    }

    /// Insert a node into the tree. Returns its NodeId.
    pub fn insert(&mut self, node: Node) -> NodeId {
        let id = node.id;
        self.nodes.insert(id.0, node);
        id
    }

    /// Get a reference to a node by id.
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id.0)
    }

    /// Get a mutable reference to a node by id.
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(&id.0)
    }

    /// Remove a node from the store (does NOT detach from parent/children).
    pub fn remove(&mut self, id: NodeId) -> Option<Node> {
        self.nodes.remove(&id.0)
    }

    /// Append `child_id` as the last child of `parent_id`.
    /// Detaches child from its current parent first if needed.
    pub fn append_child(&mut self, parent_id: NodeId, child_id: NodeId) {
        // Detach from old parent
        self.detach(child_id);

        // Set new parent
        if let Some(child) = self.nodes.get_mut(&child_id.0) {
            child.parent = parent_id;
        }

        // Add to parent's children
        if let Some(parent) = self.nodes.get_mut(&parent_id.0) {
            parent.children.push(child_id);
        }
    }

    /// Insert `child_id` before `ref_id` within `parent_id`.
    /// If `ref_id` is NULL or not found among parent's children, appends instead.
    pub fn insert_before(&mut self, parent_id: NodeId, child_id: NodeId, ref_id: NodeId) {
        if ref_id.is_null() {
            self.append_child(parent_id, child_id);
            return;
        }

        // Detach from old parent
        self.detach(child_id);

        // Set new parent
        if let Some(child) = self.nodes.get_mut(&child_id.0) {
            child.parent = parent_id;
        }

        // Insert before reference in parent's children
        if let Some(parent) = self.nodes.get_mut(&parent_id.0) {
            if let Some(pos) = parent.children.iter().position(|&c| c == ref_id) {
                parent.children.insert(pos, child_id);
            } else {
                // Reference not found, append
                parent.children.push(child_id);
            }
        }
    }

    /// Remove `child_id` from `parent_id`'s children list and clear parent ref.
    pub fn remove_child(&mut self, parent_id: NodeId, child_id: NodeId) {
        if let Some(parent) = self.nodes.get_mut(&parent_id.0) {
            parent.children.retain(|&c| c != child_id);
        }
        if let Some(child) = self.nodes.get_mut(&child_id.0) {
            child.parent = NodeId::NULL;
        }
    }

    /// Detach a node from its current parent (internal helper).
    fn detach(&mut self, child_id: NodeId) {
        let old_parent = self
            .nodes
            .get(&child_id.0)
            .map(|n| n.parent)
            .unwrap_or(NodeId::NULL);
        if !old_parent.is_null() {
            if let Some(parent) = self.nodes.get_mut(&old_parent.0) {
                parent.children.retain(|&c| c != child_id);
            }
        }
    }

    // -----------------------------------------------------------------
    // PLAT-38: focus over the whole store
    // -----------------------------------------------------------------

    /// Every node that currently holds focus, in ascending id order.
    ///
    /// **THE INSTRUMENT THE FOCUS PARTITION LAW READS**, and it walks the
    /// WHOLE store rather than answering from a remembered id, which is the
    /// only arrangement in which the law can go red. See `Node::focused`.
    pub fn focused_ids(&self) -> Vec<NodeId> {
        let mut ids: Vec<NodeId> = self
            .nodes
            .values()
            .filter(|n| n.focused)
            .map(|n| n.id)
            .collect();
        ids.sort_by_key(|i| i.0);
        ids
    }

    /// Clear focus everywhere.
    pub fn blur_all(&mut self) {
        for n in self.nodes.values_mut() {
            n.focused = false;
        }
    }

    /// Give focus to `id` and to nothing else.
    ///
    /// The clear is a separate pass over every node rather than a targeted
    /// unset of one remembered id: an implementation that unsets only the
    /// previous holder is correct exactly as long as nothing else ever set
    /// the flag, which is an assumption no assertion here could check.
    pub fn focus_only(&mut self, id: NodeId) {
        for n in self.nodes.values_mut() {
            n.focused = n.id == id;
        }
    }

    /// The node that traps focus, if any. At most one is honoured; when more
    /// than one is set the innermost-by-id wins, and `focus_trap_ids` is what
    /// lets a caller see that more than one exists.
    pub fn focus_trap_ids(&self) -> Vec<NodeId> {
        let mut ids: Vec<NodeId> = self
            .nodes
            .values()
            .filter(|n| n.focus_trap)
            .map(|n| n.id)
            .collect();
        ids.sort_by_key(|i| i.0);
        ids
    }

    /// Whether `id` is `ancestor` or sits underneath it.
    pub fn is_within(&self, id: NodeId, ancestor: NodeId) -> bool {
        if ancestor.is_null() {
            return true;
        }
        let mut cur = id;
        // The walk is bounded by the store's size, so a parent cycle
        // introduced by a future bug cannot hang the renderer here.
        for _ in 0..=self.nodes.len() {
            if cur.is_null() {
                return false;
            }
            if cur == ancestor {
                return true;
            }
            cur = self.nodes.get(&cur.0).map(|n| n.parent).unwrap_or(NodeId::NULL);
        }
        false
    }

    /// Pre-order walk from `root`, collecting the ids of focusable elements.
    /// **This IS the declared focus order** — document order, which is the
    /// order the leaf renderer built the tree in.
    pub fn focus_order(&self, root: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        self.collect_focusable(root, &mut out);
        out
    }

    fn collect_focusable(&self, id: NodeId, out: &mut Vec<NodeId>) {
        let Some(node) = self.nodes.get(&id.0) else {
            return;
        };
        if node.focusable {
            out.push(id);
        }
        for child in &node.children {
            self.collect_focusable(*child, out);
        }
    }

    /// Every node with no parent, in ascending id order. Used as the walk
    /// root when no root element has been declared.
    pub fn root_ids(&self) -> Vec<NodeId> {
        let mut ids: Vec<NodeId> = self
            .nodes
            .values()
            .filter(|n| n.parent.is_null())
            .map(|n| n.id)
            .collect();
        ids.sort_by_key(|i| i.0);
        ids
    }

    /// Get the first child of a node.
    pub fn first_child(&self, node_id: NodeId) -> NodeId {
        self.nodes
            .get(&node_id.0)
            .and_then(|n| n.children.first().copied())
            .unwrap_or(NodeId::NULL)
    }

    /// Get the next sibling of a node (the node after it in its parent's children list).
    pub fn next_sibling(&self, node_id: NodeId) -> NodeId {
        let parent_id = self
            .nodes
            .get(&node_id.0)
            .map(|n| n.parent)
            .unwrap_or(NodeId::NULL);
        if parent_id.is_null() {
            return NodeId::NULL;
        }
        if let Some(parent) = self.nodes.get(&parent_id.0) {
            if let Some(pos) = parent.children.iter().position(|&c| c == node_id) {
                if pos + 1 < parent.children.len() {
                    return parent.children[pos + 1];
                }
            }
        }
        NodeId::NULL
    }

    /// Get the parent of a node.
    pub fn parent_node(&self, node_id: NodeId) -> NodeId {
        self.nodes
            .get(&node_id.0)
            .map(|n| n.parent)
            .unwrap_or(NodeId::NULL)
    }

    /// Get the total number of nodes in the tree.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Check if the tree is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Recursively collect text content from a node and its descendants.
    pub fn collect_text(&self, node_id: NodeId) -> String {
        if let Some(node) = self.get(node_id) {
            match &node.kind {
                NodeKind::Text(text) => text.clone(),
                NodeKind::Element(_) => {
                    // Check for __text_content attribute (set by setTextContent on elements)
                    if let Some(tc) = node.attributes.get("__text_content") {
                        return tc.clone();
                    }
                    let mut result = String::new();
                    for &child_id in &node.children {
                        result.push_str(&self.collect_text(child_id));
                    }
                    result
                }
            }
        } else {
            String::new()
        }
    }
}

impl Default for Tree {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_element() {
        let node = Node::new_element("div");
        assert_eq!(node.tag(), Some("div"));
        assert!(node.text_content().is_none());
        assert!(!node.id.is_null());
        assert_eq!(node.gpui_kind, GpuiElementKind::Div);
    }

    #[test]
    fn test_create_text() {
        let node = Node::new_text("hello");
        assert!(node.tag().is_none());
        assert_eq!(node.text_content(), Some("hello"));
        assert_eq!(node.gpui_kind, GpuiElementKind::TextNode);
    }

    #[test]
    fn test_append_child() {
        let mut tree = Tree::new();
        let parent = Node::new_element("div");
        let child = Node::new_element("span");
        let pid = tree.insert(parent);
        let cid = tree.insert(child);

        tree.append_child(pid, cid);

        assert_eq!(tree.get(pid).unwrap().children, vec![cid]);
        assert_eq!(tree.get(cid).unwrap().parent, pid);
    }

    #[test]
    fn test_insert_before() {
        let mut tree = Tree::new();
        let parent = Node::new_element("div");
        let c1 = Node::new_element("span");
        let c2 = Node::new_element("span");
        let c3 = Node::new_element("span");
        let pid = tree.insert(parent);
        let c1id = tree.insert(c1);
        let c2id = tree.insert(c2);
        let c3id = tree.insert(c3);

        tree.append_child(pid, c1id);
        tree.append_child(pid, c2id);
        // Insert c3 before c2
        tree.insert_before(pid, c3id, c2id);

        assert_eq!(tree.get(pid).unwrap().children, vec![c1id, c3id, c2id]);
    }

    #[test]
    fn test_insert_before_null_ref_appends() {
        let mut tree = Tree::new();
        let parent = Node::new_element("div");
        let c1 = Node::new_element("span");
        let c2 = Node::new_element("span");
        let pid = tree.insert(parent);
        let c1id = tree.insert(c1);
        let c2id = tree.insert(c2);

        tree.append_child(pid, c1id);
        tree.insert_before(pid, c2id, NodeId::NULL);

        assert_eq!(tree.get(pid).unwrap().children, vec![c1id, c2id]);
    }

    #[test]
    fn test_remove_child() {
        let mut tree = Tree::new();
        let parent = Node::new_element("div");
        let child = Node::new_element("span");
        let pid = tree.insert(parent);
        let cid = tree.insert(child);

        tree.append_child(pid, cid);
        tree.remove_child(pid, cid);

        assert!(tree.get(pid).unwrap().children.is_empty());
        assert_eq!(tree.get(cid).unwrap().parent, NodeId::NULL);
    }

    #[test]
    fn test_first_child_next_sibling() {
        let mut tree = Tree::new();
        let parent = Node::new_element("div");
        let c1 = Node::new_element("span");
        let c2 = Node::new_element("span");
        let pid = tree.insert(parent);
        let c1id = tree.insert(c1);
        let c2id = tree.insert(c2);

        tree.append_child(pid, c1id);
        tree.append_child(pid, c2id);

        assert_eq!(tree.first_child(pid), c1id);
        assert_eq!(tree.next_sibling(c1id), c2id);
        assert_eq!(tree.next_sibling(c2id), NodeId::NULL);
    }

    #[test]
    fn test_parent_node() {
        let mut tree = Tree::new();
        let parent = Node::new_element("div");
        let child = Node::new_element("span");
        let pid = tree.insert(parent);
        let cid = tree.insert(child);

        tree.append_child(pid, cid);

        assert_eq!(tree.parent_node(cid), pid);
        assert_eq!(tree.parent_node(pid), NodeId::NULL);
    }

    #[test]
    fn test_set_attributes_and_styles() {
        let mut tree = Tree::new();
        let mut node = Node::new_element("div");
        node.attributes.insert("width".into(), "100%".into());
        node.styles.insert("background".into(), "red".into());
        let id = tree.insert(node);

        let n = tree.get(id).unwrap();
        assert_eq!(n.attributes.get("width").map(|s| s.as_str()), Some("100%"));
        assert_eq!(n.styles.get("background").map(|s| s.as_str()), Some("red"));
    }

    #[test]
    fn test_set_text_content_text_node() {
        let mut node = Node::new_text("hello");
        node.set_text_content("world");
        assert_eq!(node.text_content(), Some("world"));
    }

    #[test]
    fn test_set_text_content_element_node() {
        let mut node = Node::new_element("div");
        node.set_text_content("hello");
        assert_eq!(
            node.attributes.get("__text_content").map(|s| s.as_str()),
            Some("hello")
        );
    }

    #[test]
    fn test_reparent_detaches_from_old() {
        let mut tree = Tree::new();
        let p1 = Node::new_element("div");
        let p2 = Node::new_element("div");
        let child = Node::new_element("span");
        let p1id = tree.insert(p1);
        let p2id = tree.insert(p2);
        let cid = tree.insert(child);

        tree.append_child(p1id, cid);
        assert_eq!(tree.get(p1id).unwrap().children.len(), 1);

        // Reparent to p2
        tree.append_child(p2id, cid);
        assert!(tree.get(p1id).unwrap().children.is_empty());
        assert_eq!(tree.get(p2id).unwrap().children, vec![cid]);
        assert_eq!(tree.get(cid).unwrap().parent, p2id);
    }

    #[test]
    fn test_reparent_via_insert_before() {
        let mut tree = Tree::new();
        let p1 = Node::new_element("div");
        let p2 = Node::new_element("div");
        let c1 = Node::new_element("span");
        let c2 = Node::new_element("span");
        let p1id = tree.insert(p1);
        let p2id = tree.insert(p2);
        let c1id = tree.insert(c1);
        let c2id = tree.insert(c2);

        tree.append_child(p1id, c1id);
        tree.append_child(p2id, c2id);
        // Move c1 to p2 before c2
        tree.insert_before(p2id, c1id, c2id);

        assert!(tree.get(p1id).unwrap().children.is_empty());
        assert_eq!(tree.get(p2id).unwrap().children, vec![c1id, c2id]);
        assert_eq!(tree.get(c1id).unwrap().parent, p2id);
    }

    #[test]
    fn test_collect_text_simple() {
        let mut tree = Tree::new();
        let node = Node::new_text("hello");
        let id = tree.insert(node);
        assert_eq!(tree.collect_text(id), "hello");
    }

    #[test]
    fn test_collect_text_recursive() {
        let mut tree = Tree::new();
        let parent = Node::new_element("div");
        let t1 = Node::new_text("hello ");
        let t2 = Node::new_text("world");
        let pid = tree.insert(parent);
        let t1id = tree.insert(t1);
        let t2id = tree.insert(t2);
        tree.append_child(pid, t1id);
        tree.append_child(pid, t2id);
        assert_eq!(tree.collect_text(pid), "hello world");
    }

    #[test]
    fn test_collect_text_with_text_content_attr() {
        let mut tree = Tree::new();
        let mut node = Node::new_element("div");
        node.set_text_content("direct text");
        let id = tree.insert(node);
        assert_eq!(tree.collect_text(id), "direct text");
    }

    #[test]
    fn test_tree_len() {
        let mut tree = Tree::new();
        assert_eq!(tree.len(), 0);
        assert!(tree.is_empty());

        let n1 = Node::new_element("div");
        tree.insert(n1);
        assert_eq!(tree.len(), 1);
        assert!(!tree.is_empty());
    }

    #[test]
    fn test_remove_node() {
        let mut tree = Tree::new();
        let node = Node::new_element("div");
        let id = tree.insert(node);
        assert!(tree.get(id).is_some());
        tree.remove(id);
        assert!(tree.get(id).is_none());
    }

    #[test]
    fn test_first_child_empty() {
        let mut tree = Tree::new();
        let parent = Node::new_element("div");
        let pid = tree.insert(parent);
        assert_eq!(tree.first_child(pid), NodeId::NULL);
    }

    #[test]
    fn test_next_sibling_no_parent() {
        let mut tree = Tree::new();
        let node = Node::new_element("div");
        let id = tree.insert(node);
        assert_eq!(tree.next_sibling(id), NodeId::NULL);
    }

    #[test]
    fn test_tag_to_gpui_kind_mapping() {
        // Container elements -> Div
        assert_eq!(tag_to_gpui_kind("div"), GpuiElementKind::Div);
        assert_eq!(tag_to_gpui_kind("section"), GpuiElementKind::Div);
        assert_eq!(tag_to_gpui_kind("article"), GpuiElementKind::Div);
        assert_eq!(tag_to_gpui_kind("nav"), GpuiElementKind::Div);
        assert_eq!(tag_to_gpui_kind("header"), GpuiElementKind::Div);
        assert_eq!(tag_to_gpui_kind("footer"), GpuiElementKind::Div);
        assert_eq!(tag_to_gpui_kind("button"), GpuiElementKind::Div);
        assert_eq!(tag_to_gpui_kind("input"), GpuiElementKind::Div);
        assert_eq!(tag_to_gpui_kind("ul"), GpuiElementKind::Div);
        assert_eq!(tag_to_gpui_kind("ol"), GpuiElementKind::Div);
        assert_eq!(tag_to_gpui_kind("li"), GpuiElementKind::Div);
        assert_eq!(tag_to_gpui_kind("form"), GpuiElementKind::Div);

        // Text-oriented elements -> TextContainer
        assert_eq!(tag_to_gpui_kind("span"), GpuiElementKind::TextContainer);
        assert_eq!(tag_to_gpui_kind("p"), GpuiElementKind::TextContainer);
        assert_eq!(tag_to_gpui_kind("h1"), GpuiElementKind::TextContainer);
        assert_eq!(tag_to_gpui_kind("h2"), GpuiElementKind::TextContainer);
        assert_eq!(tag_to_gpui_kind("h3"), GpuiElementKind::TextContainer);
        assert_eq!(tag_to_gpui_kind("h4"), GpuiElementKind::TextContainer);
        assert_eq!(tag_to_gpui_kind("h5"), GpuiElementKind::TextContainer);
        assert_eq!(tag_to_gpui_kind("h6"), GpuiElementKind::TextContainer);
        assert_eq!(tag_to_gpui_kind("label"), GpuiElementKind::TextContainer);
        assert_eq!(tag_to_gpui_kind("strong"), GpuiElementKind::TextContainer);
        assert_eq!(tag_to_gpui_kind("em"), GpuiElementKind::TextContainer);
        assert_eq!(tag_to_gpui_kind("code"), GpuiElementKind::TextContainer);

        // Media elements
        assert_eq!(tag_to_gpui_kind("img"), GpuiElementKind::Img);
        assert_eq!(tag_to_gpui_kind("svg"), GpuiElementKind::Svg);

        // Text node
        assert_eq!(tag_to_gpui_kind("#text"), GpuiElementKind::TextNode);

        // Unknown defaults to Div
        assert_eq!(tag_to_gpui_kind("custom-element"), GpuiElementKind::Div);
    }

    #[test]
    fn test_event_listeners() {
        let mut node = Node::new_element("button");
        extern "C" fn handler1(_p: *const crate::input::GpuiEventPayload) {}
        extern "C" fn handler2(_p: *const crate::input::GpuiEventPayload) {}

        node.event_listeners
            .entry("click".to_string())
            .or_default()
            .push(EventListener {
                callback: handler1,
                callback_id: 0,
            });
        node.event_listeners
            .entry("click".to_string())
            .or_default()
            .push(EventListener {
                callback: handler2,
                callback_id: 0,
            });
        node.event_listeners
            .entry("mousedown".to_string())
            .or_default()
            .push(EventListener {
                callback: handler1,
                callback_id: 0,
            });

        assert_eq!(node.event_listeners.get("click").unwrap().len(), 2);
        assert_eq!(node.event_listeners.get("mousedown").unwrap().len(), 1);
        assert!(node.event_listeners.get("keydown").is_none());
    }

    #[test]
    fn test_multiple_children_ordering() {
        let mut tree = Tree::new();
        let parent = Node::new_element("div");
        let pid = tree.insert(parent);

        let mut child_ids = Vec::new();
        for i in 0..5 {
            let child = Node::new_element(&format!("child{}", i));
            let cid = tree.insert(child);
            tree.append_child(pid, cid);
            child_ids.push(cid);
        }

        let parent_node = tree.get(pid).unwrap();
        assert_eq!(parent_node.children, child_ids);

        // Verify traversal
        let mut current = tree.first_child(pid);
        for expected in &child_ids {
            assert_eq!(current, *expected);
            current = tree.next_sibling(current);
        }
        assert_eq!(current, NodeId::NULL);
    }

    #[test]
    fn test_remove_middle_child() {
        let mut tree = Tree::new();
        let parent = Node::new_element("div");
        let c1 = Node::new_element("span");
        let c2 = Node::new_element("span");
        let c3 = Node::new_element("span");
        let pid = tree.insert(parent);
        let c1id = tree.insert(c1);
        let c2id = tree.insert(c2);
        let c3id = tree.insert(c3);

        tree.append_child(pid, c1id);
        tree.append_child(pid, c2id);
        tree.append_child(pid, c3id);

        // Remove middle child
        tree.remove_child(pid, c2id);

        assert_eq!(tree.get(pid).unwrap().children, vec![c1id, c3id]);
        assert_eq!(tree.next_sibling(c1id), c3id);
        assert_eq!(tree.next_sibling(c3id), NodeId::NULL);
    }

    #[test]
    fn test_node_id_uniqueness() {
        let id1 = NodeId::new();
        let id2 = NodeId::new();
        let id3 = NodeId::new();
        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_node_id_null() {
        assert!(NodeId::NULL.is_null());
        assert_eq!(NodeId::NULL.0, 0);
        let id = NodeId::new();
        assert!(!id.is_null());
    }
}
