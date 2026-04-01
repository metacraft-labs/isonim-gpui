//! Render-sync bridge: translates the shadow element tree into GPUI's declarative model.
//!
//! This module is the core of the imperative->declarative bridge. It walks the
//! shadow tree maintained by the C FFI layer and produces a render plan describing
//! what GPUI elements to create.
//!
//! ## Mapping
//!
//! | Shadow tag      | GPUI element |
//! |-----------------|--------------|
//! | `div`, `section`, `header`, etc. | `div()` |
//! | `span`, `p`, `h1`-`h6`, etc.    | `div()` + text child (TextContainer) |
//! | `button`        | `div()` (with click handler) |
//! | `img`           | `img()` |
//! | `svg`           | `svg()` |
//! | `root`          | `div()` (full-size container) |
//! | text node       | string child of parent div |
//! | unknown tag     | `div()` (fallback container) |
//!
//! ## Style mapping
//!
//! Shadow style properties use CSS-like names. This module maps them to
//! GPUI's Tailwind-inspired builder methods:
//! - `background-color` -> `.bg()`
//! - `width` / `height` -> `.w()` / `.h()`
//! - `padding` / `margin` -> `.p()` / `.m()`
//! - `color` -> `.text_color()`
//! - `font-size` -> `.text_size()` (custom px)
//! - `border-radius` -> `.rounded()`
//! - `flex-direction` -> `.flex_row()` / `.flex_col()`
//! - `align-items` -> `.items_center()` etc.
//! - `justify-content` -> `.justify_center()` etc.
//! - `gap` -> `.gap()`

use crate::tree::{GpuiElementKind, Node, NodeId, NodeKind, Tree};

/// Determines what GPUI element a shadow node should map to.
pub fn classify_node(node: &Node) -> GpuiElementKind {
    match &node.kind {
        NodeKind::Text(_) => GpuiElementKind::TextNode,
        NodeKind::Element(tag) => classify_tag(tag),
    }
}

/// Maps a shadow tag name to a GPUI element kind.
/// This matches the `tag_to_gpui_kind` in `tree.rs` but is used by
/// the render-sync layer for consistency.
pub fn classify_tag(tag: &str) -> GpuiElementKind {
    crate::tree::tag_to_gpui_kind(tag)
}

/// A collected style ready to be applied to a GPUI element.
/// GPUI uses Tailwind-inspired builder methods; this struct normalizes
/// CSS-like property names to GPUI equivalents.
#[derive(Debug, Clone, Default)]
pub struct GpuiStyles {
    pub bg: Option<String>,
    pub w: Option<String>,
    pub h: Option<String>,
    pub min_w: Option<String>,
    pub min_h: Option<String>,
    pub max_w: Option<String>,
    pub max_h: Option<String>,
    pub p: Option<String>,
    pub m: Option<String>,
    pub flex_direction: Option<String>,
    pub gap: Option<String>,
    pub text_size: Option<String>,
    pub text_color: Option<String>,
    pub rounded: Option<String>,
    pub items: Option<String>,
    pub justify: Option<String>,
    pub border_width: Option<String>,
    pub border_color: Option<String>,
    pub shadow: Option<String>,
    pub opacity: Option<String>,
    pub overflow: Option<String>,
    pub font_family: Option<String>,
    pub font_weight: Option<String>,
    pub font_style: Option<String>,
    pub line_height: Option<String>,
    pub letter_spacing: Option<String>,
    pub cursor: Option<String>,
    pub display: Option<String>,
    pub position: Option<String>,
}

impl GpuiStyles {
    /// Collect styles from a shadow node, normalizing CSS-like names to GPUI names.
    pub fn from_node(node: &Node) -> Self {
        let mut s = GpuiStyles::default();
        for (key, value) in &node.styles {
            s.apply(key, value);
        }
        // Also check attributes that might be styling (e.g. width/height set as attributes)
        for (key, value) in &node.attributes {
            // Skip internal attributes
            if key.starts_with("__") {
                continue;
            }
            // Only apply attribute-based styles if no explicit style was set
            s.apply_if_absent(key, value);
        }
        s
    }

    /// Apply a single CSS-like property.
    fn apply(&mut self, key: &str, value: &str) {
        let v = value.to_string();
        match key {
            "background" | "background-color" | "background_color" | "bg" => self.bg = Some(v),
            "width" | "w" => self.w = Some(v),
            "height" | "h" => self.h = Some(v),
            "min-width" | "min_width" | "min_w" => self.min_w = Some(v),
            "min-height" | "min_height" | "min_h" => self.min_h = Some(v),
            "max-width" | "max_width" | "max_w" => self.max_w = Some(v),
            "max-height" | "max_height" | "max_h" => self.max_h = Some(v),
            "padding" | "p" => self.p = Some(v),
            "margin" | "m" => self.m = Some(v),
            "flex-direction" | "flex_direction" => {
                self.flex_direction = Some(normalize_flex_direction(value))
            }
            "gap" => self.gap = Some(v),
            "font-size" | "font_size" | "text_size" => self.text_size = Some(v),
            "color" | "text-color" | "text_color" => self.text_color = Some(v),
            "border-radius" | "border_radius" | "rounded" | "corner-radius" | "corner_radius" => {
                self.rounded = Some(v)
            }
            "align-items" | "align_items" | "items" => {
                self.items = Some(normalize_alignment(value))
            }
            "justify-content" | "justify_content" | "justify" => {
                self.justify = Some(normalize_justification(value))
            }
            "border-width" | "border_width" | "border" => self.border_width = Some(v),
            "border-color" | "border_color" => self.border_color = Some(v),
            "shadow" | "box-shadow" | "box_shadow" => self.shadow = Some(v),
            "opacity" => self.opacity = Some(v),
            "overflow" => self.overflow = Some(v),
            "font-family" | "font_family" => self.font_family = Some(v),
            "font-weight" | "font_weight" => self.font_weight = Some(v),
            "font-style" | "font_style" => self.font_style = Some(v),
            "line-height" | "line_height" => self.line_height = Some(v),
            "letter-spacing" | "letter_spacing" => self.letter_spacing = Some(v),
            "cursor" => self.cursor = Some(v),
            "display" => self.display = Some(v),
            "position" => self.position = Some(v),
            _ => {} // Ignore unknown properties
        }
    }

    /// Apply a property only if the corresponding field is None.
    fn apply_if_absent(&mut self, key: &str, value: &str) {
        let already_set = match key {
            "width" | "w" => self.w.is_some(),
            "height" | "h" => self.h.is_some(),
            "padding" | "p" => self.p.is_some(),
            "margin" | "m" => self.m.is_some(),
            "background" | "background-color" | "background_color" | "bg" => self.bg.is_some(),
            "color" | "text-color" | "text_color" => self.text_color.is_some(),
            _ => false,
        };
        if !already_set {
            self.apply(key, value);
        }
    }
}

/// Normalize CSS flex-direction values to canonical names.
fn normalize_flex_direction(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "row" | "horizontal" => "row".to_string(),
        "column" | "col" | "vertical" => "column".to_string(),
        other => other.to_string(),
    }
}

/// Normalize CSS align-items values.
fn normalize_alignment(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "center" => "center".to_string(),
        "start" | "flex-start" => "start".to_string(),
        "end" | "flex-end" => "end".to_string(),
        "baseline" => "baseline".to_string(),
        "stretch" => "stretch".to_string(),
        other => other.to_string(),
    }
}

/// Normalize CSS justify-content values.
fn normalize_justification(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "center" => "center".to_string(),
        "start" | "flex-start" => "start".to_string(),
        "end" | "flex-end" => "end".to_string(),
        "space-between" | "space_between" => "space_between".to_string(),
        "space-around" | "space_around" => "space_around".to_string(),
        other => other.to_string(),
    }
}

/// Describes a single node in the render plan -- an intermediate representation
/// between the shadow tree and GPUI elements. This allows testing the mapping
/// logic without requiring actual GPUI rendering.
#[derive(Debug, Clone)]
pub struct RenderNode {
    /// The shadow node ID this render node corresponds to.
    pub node_id: u64,
    /// What kind of GPUI element to produce.
    pub kind: GpuiElementKind,
    /// The original HTML tag (for debugging/introspection).
    pub tag: String,
    /// Collected and normalized styles.
    pub styles: GpuiStyles,
    /// Text content (for text nodes or elements with __text_content).
    pub text: Option<String>,
    /// Whether this node has a "click" event listener.
    pub has_click_handler: bool,
    /// Whether this node has an "input" event listener (keyboard input).
    pub has_input_handler: bool,
    /// All event listener names attached to this node (for introspection/testing).
    pub event_names: Vec<String>,
    /// Children render nodes (recursive).
    pub children: Vec<RenderNode>,
}

/// Build a render plan from the shadow tree, starting at `root_id`.
///
/// This walks the tree recursively and produces a `RenderNode` hierarchy
/// that describes what GPUI elements to create. This intermediate
/// representation can be tested without GPUI dependencies.
pub fn build_render_plan(tree: &Tree, root_id: NodeId) -> Option<RenderNode> {
    let node = tree.get(root_id)?;
    let kind = classify_node(node);
    let styles = GpuiStyles::from_node(node);

    let tag = match &node.kind {
        NodeKind::Element(t) => t.clone(),
        NodeKind::Text(_) => "#text".to_string(),
    };

    let text = match &node.kind {
        NodeKind::Text(t) => Some(t.clone()),
        NodeKind::Element(_) => node.attributes.get("__text_content").cloned(),
    };

    let has_click_handler = node.event_listeners.contains_key("click");
    let has_input_handler = node.event_listeners.contains_key("input");
    let event_names: Vec<String> = node.event_listeners.keys().cloned().collect();

    let children: Vec<RenderNode> = node
        .children
        .iter()
        .filter_map(|&child_id| build_render_plan(tree, child_id))
        .collect();

    Some(RenderNode {
        node_id: root_id.0,
        kind,
        tag,
        styles,
        text,
        has_click_handler,
        has_input_handler,
        event_names,
        children,
    })
}

/// Count total render nodes in a plan (for testing).
pub fn count_render_nodes(plan: &RenderNode) -> usize {
    1 + plan
        .children
        .iter()
        .map(count_render_nodes)
        .sum::<usize>()
}

// ---------------------------------------------------------------------------
// GPUI rendering (only available with gpui-backend feature)
// ---------------------------------------------------------------------------

#[cfg(feature = "gpui-backend")]
pub mod gpui_render {
    //! Actual GPUI element rendering from the shadow tree.
    //!
    //! This module contains the rendering functions that read the global shadow
    //! tree and produce GPUI elements via builder chains on each render cycle.

    use super::*;

    /// Dispatch an event to the shadow tree's event listeners.
    /// This is called from GPUI event handlers to bridge back to the
    /// Nim-side callbacks.
    ///
    /// The lock is released before calling callbacks to avoid deadlocks
    /// (callbacks may mutate the tree via FFI).
    pub fn dispatch_shadow_event(node_id: NodeId, event_name: &str) {
        let callbacks: Vec<extern "C" fn()> = {
            let tree = crate::lock_tree();
            if let Some(node) = tree.get(node_id) {
                node.event_listeners
                    .get(event_name)
                    .map(|listeners| listeners.iter().map(|l| l.callback).collect())
                    .unwrap_or_default()
            } else {
                Vec::new()
            }
        };
        // Call callbacks outside the lock to avoid deadlocks
        for cb in callbacks {
            cb();
        }
    }
}

// ---------------------------------------------------------------------------
// Tests (no GPUI dependency required)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{EventListener, Node, NodeId, Tree};

    #[test]
    fn test_classify_div() {
        assert_eq!(classify_tag("div"), GpuiElementKind::Div);
    }

    #[test]
    fn test_classify_root() {
        assert_eq!(classify_tag("root"), GpuiElementKind::Div);
    }

    #[test]
    fn test_classify_button() {
        assert_eq!(classify_tag("button"), GpuiElementKind::Div);
    }

    #[test]
    fn test_classify_span() {
        assert_eq!(classify_tag("span"), GpuiElementKind::TextContainer);
    }

    #[test]
    fn test_classify_p() {
        assert_eq!(classify_tag("p"), GpuiElementKind::TextContainer);
    }

    #[test]
    fn test_classify_h1() {
        assert_eq!(classify_tag("h1"), GpuiElementKind::TextContainer);
    }

    #[test]
    fn test_classify_img() {
        assert_eq!(classify_tag("img"), GpuiElementKind::Img);
    }

    #[test]
    fn test_classify_svg() {
        assert_eq!(classify_tag("svg"), GpuiElementKind::Svg);
    }

    #[test]
    fn test_classify_unknown_defaults_to_div() {
        assert_eq!(classify_tag("custom-component"), GpuiElementKind::Div);
    }

    #[test]
    fn test_classify_text_node() {
        let node = Node::new_text("hello");
        assert_eq!(classify_node(&node), GpuiElementKind::TextNode);
    }

    #[test]
    fn test_classify_element_node() {
        let node = Node::new_element("div");
        assert_eq!(classify_node(&node), GpuiElementKind::Div);
    }

    #[test]
    fn test_styles_from_node_css_properties() {
        let mut node = Node::new_element("div");
        node.styles.insert("width".into(), "100%".into());
        node.styles.insert("height".into(), "50px".into());
        node.styles
            .insert("background-color".into(), "red".into());
        node.styles.insert("font-size".into(), "16".into());
        node.styles.insert("border-radius".into(), "8".into());
        node.styles.insert("color".into(), "#ffffff".into());
        node.styles.insert("gap".into(), "10".into());
        node.styles.insert("padding".into(), "5".into());
        node.styles.insert("margin".into(), "3".into());

        let styles = GpuiStyles::from_node(&node);
        assert_eq!(styles.w.as_deref(), Some("100%"));
        assert_eq!(styles.h.as_deref(), Some("50px"));
        assert_eq!(styles.bg.as_deref(), Some("red"));
        assert_eq!(styles.text_size.as_deref(), Some("16"));
        assert_eq!(styles.rounded.as_deref(), Some("8"));
        assert_eq!(styles.text_color.as_deref(), Some("#ffffff"));
        assert_eq!(styles.gap.as_deref(), Some("10"));
        assert_eq!(styles.p.as_deref(), Some("5"));
        assert_eq!(styles.m.as_deref(), Some("3"));
    }

    #[test]
    fn test_styles_flex_direction_normalization() {
        let mut node = Node::new_element("div");
        node.styles.insert("flex-direction".into(), "row".into());
        let styles = GpuiStyles::from_node(&node);
        assert_eq!(styles.flex_direction.as_deref(), Some("row"));

        let mut node2 = Node::new_element("div");
        node2.styles.insert("flex-direction".into(), "column".into());
        let styles2 = GpuiStyles::from_node(&node2);
        assert_eq!(styles2.flex_direction.as_deref(), Some("column"));

        let mut node3 = Node::new_element("div");
        node3.styles.insert("flex-direction".into(), "col".into());
        let styles3 = GpuiStyles::from_node(&node3);
        assert_eq!(styles3.flex_direction.as_deref(), Some("column"));
    }

    #[test]
    fn test_styles_alignment_normalization() {
        let mut node = Node::new_element("div");
        node.styles.insert("align-items".into(), "center".into());
        node.styles
            .insert("justify-content".into(), "space-between".into());
        let styles = GpuiStyles::from_node(&node);
        assert_eq!(styles.items.as_deref(), Some("center"));
        assert_eq!(styles.justify.as_deref(), Some("space_between"));
    }

    #[test]
    fn test_styles_attribute_fallback() {
        let mut node = Node::new_element("div");
        node.attributes.insert("width".into(), "200".into());
        node.attributes.insert("height".into(), "100".into());
        let styles = GpuiStyles::from_node(&node);
        assert_eq!(styles.w.as_deref(), Some("200"));
        assert_eq!(styles.h.as_deref(), Some("100"));
    }

    #[test]
    fn test_styles_explicit_style_overrides_attribute() {
        let mut node = Node::new_element("div");
        node.styles.insert("width".into(), "300".into());
        node.attributes.insert("width".into(), "200".into());
        let styles = GpuiStyles::from_node(&node);
        assert_eq!(styles.w.as_deref(), Some("300"));
    }

    #[test]
    fn test_styles_internal_attributes_ignored() {
        let mut node = Node::new_element("div");
        node.attributes
            .insert("__text_content".into(), "hello".into());
        let styles = GpuiStyles::from_node(&node);
        assert!(styles.w.is_none());
        assert!(styles.bg.is_none());
    }

    #[test]
    fn test_build_render_plan_single_node() {
        let mut tree = Tree::new();
        let node = Node::new_element("div");
        let id = tree.insert(node);

        let plan = build_render_plan(&tree, id).unwrap();
        assert_eq!(plan.node_id, id.0);
        assert_eq!(plan.kind, GpuiElementKind::Div);
        assert_eq!(plan.tag, "div");
        assert!(plan.children.is_empty());
        assert!(plan.text.is_none());
        assert!(!plan.has_click_handler);
    }

    #[test]
    fn test_build_render_plan_text_node() {
        let mut tree = Tree::new();
        let node = Node::new_text("hello world");
        let id = tree.insert(node);

        let plan = build_render_plan(&tree, id).unwrap();
        assert_eq!(plan.kind, GpuiElementKind::TextNode);
        assert_eq!(plan.tag, "#text");
        assert_eq!(plan.text.as_deref(), Some("hello world"));
    }

    #[test]
    fn test_build_render_plan_with_children() {
        let mut tree = Tree::new();
        let root = Node::new_element("root");
        let child1 = Node::new_element("span");
        let child2 = Node::new_text("text");

        let root_id = tree.insert(root);
        let c1_id = tree.insert(child1);
        let c2_id = tree.insert(child2);

        tree.append_child(root_id, c1_id);
        tree.append_child(root_id, c2_id);

        let plan = build_render_plan(&tree, root_id).unwrap();
        assert_eq!(plan.children.len(), 2);
        assert_eq!(plan.children[0].kind, GpuiElementKind::TextContainer);
        assert_eq!(plan.children[1].kind, GpuiElementKind::TextNode);
        assert_eq!(plan.children[1].text.as_deref(), Some("text"));
    }

    #[test]
    fn test_build_render_plan_with_styles() {
        let mut tree = Tree::new();
        let mut node = Node::new_element("div");
        node.styles.insert("width".into(), "100%".into());
        node.styles.insert("background".into(), "red".into());
        let id = tree.insert(node);

        let plan = build_render_plan(&tree, id).unwrap();
        assert_eq!(plan.styles.w.as_deref(), Some("100%"));
        assert_eq!(plan.styles.bg.as_deref(), Some("red"));
    }

    #[test]
    fn test_build_render_plan_with_click_handler() {
        let mut tree = Tree::new();
        let mut node = Node::new_element("button");
        extern "C" fn noop() {}
        node.event_listeners
            .entry("click".into())
            .or_default()
            .push(EventListener { callback: noop });
        let id = tree.insert(node);

        let plan = build_render_plan(&tree, id).unwrap();
        assert!(plan.has_click_handler);
        assert_eq!(plan.kind, GpuiElementKind::Div);
    }

    #[test]
    fn test_build_render_plan_nonexistent_node() {
        let tree = Tree::new();
        let plan = build_render_plan(&tree, NodeId(999));
        assert!(plan.is_none());
    }

    #[test]
    fn test_build_render_plan_deep_tree() {
        let mut tree = Tree::new();
        let root = Node::new_element("root");
        let div = Node::new_element("div");
        let span = Node::new_element("span");
        let text = Node::new_text("nested");

        let root_id = tree.insert(root);
        let div_id = tree.insert(div);
        let span_id = tree.insert(span);
        let text_id = tree.insert(text);

        tree.append_child(root_id, div_id);
        tree.append_child(div_id, span_id);
        tree.append_child(span_id, text_id);

        let plan = build_render_plan(&tree, root_id).unwrap();
        assert_eq!(count_render_nodes(&plan), 4);
        assert_eq!(plan.children.len(), 1); // div
        assert_eq!(plan.children[0].children.len(), 1); // span
        assert_eq!(plan.children[0].children[0].children.len(), 1); // text
        assert_eq!(
            plan.children[0].children[0].children[0].text.as_deref(),
            Some("nested")
        );
    }

    #[test]
    fn test_build_render_plan_element_with_text_content() {
        let mut tree = Tree::new();
        let mut node = Node::new_element("div");
        node.set_text_content("direct text");
        let id = tree.insert(node);

        let plan = build_render_plan(&tree, id).unwrap();
        assert_eq!(plan.text.as_deref(), Some("direct text"));
    }

    #[test]
    fn test_count_render_nodes() {
        let plan = RenderNode {
            node_id: 1,
            kind: GpuiElementKind::Div,
            tag: "div".into(),
            styles: GpuiStyles::default(),
            text: None,
            has_click_handler: false,
            has_input_handler: false,
            event_names: vec![],
            children: vec![
                RenderNode {
                    node_id: 2,
                    kind: GpuiElementKind::TextContainer,
                    tag: "span".into(),
                    styles: GpuiStyles::default(),
                    text: Some("a".into()),
                    has_click_handler: false,
                    has_input_handler: false,
                    event_names: vec![],
                    children: vec![],
                },
                RenderNode {
                    node_id: 3,
                    kind: GpuiElementKind::Div,
                    tag: "div".into(),
                    styles: GpuiStyles::default(),
                    text: None,
                    has_click_handler: false,
                    has_input_handler: false,
                    event_names: vec![],
                    children: vec![RenderNode {
                        node_id: 4,
                        kind: GpuiElementKind::TextNode,
                        tag: "#text".into(),
                        styles: GpuiStyles::default(),
                        text: Some("b".into()),
                        has_click_handler: false,
                        has_input_handler: false,
                        event_names: vec![],
                        children: vec![],
                    }],
                },
            ],
        };
        assert_eq!(count_render_nodes(&plan), 4);
    }

    #[test]
    fn test_build_render_plan_with_input_handler() {
        let mut tree = Tree::new();
        let mut node = Node::new_element("div");
        extern "C" fn noop() {}
        node.event_listeners
            .entry("input".into())
            .or_default()
            .push(EventListener { callback: noop });
        let id = tree.insert(node);

        let plan = build_render_plan(&tree, id).unwrap();
        assert!(plan.has_input_handler);
        assert!(!plan.has_click_handler);
        assert!(plan.event_names.contains(&"input".to_string()));
    }

    #[test]
    fn test_build_render_plan_multiple_event_handlers() {
        let mut tree = Tree::new();
        let mut node = Node::new_element("button");
        extern "C" fn noop() {}
        node.event_listeners
            .entry("click".into())
            .or_default()
            .push(EventListener { callback: noop });
        node.event_listeners
            .entry("input".into())
            .or_default()
            .push(EventListener { callback: noop });
        node.event_listeners
            .entry("hover".into())
            .or_default()
            .push(EventListener { callback: noop });
        let id = tree.insert(node);

        let plan = build_render_plan(&tree, id).unwrap();
        assert!(plan.has_click_handler);
        assert!(plan.has_input_handler);
        assert_eq!(plan.event_names.len(), 3);
        assert!(plan.event_names.contains(&"click".to_string()));
        assert!(plan.event_names.contains(&"input".to_string()));
        assert!(plan.event_names.contains(&"hover".to_string()));
    }

    #[test]
    fn test_build_render_plan_no_handlers_by_default() {
        let mut tree = Tree::new();
        let node = Node::new_element("div");
        let id = tree.insert(node);

        let plan = build_render_plan(&tree, id).unwrap();
        assert!(!plan.has_click_handler);
        assert!(!plan.has_input_handler);
        assert!(plan.event_names.is_empty());
    }

    #[test]
    fn test_render_plan_preserves_event_handlers_in_children() {
        let mut tree = Tree::new();
        extern "C" fn noop() {}

        let root = Node::new_element("root");
        let root_id = tree.insert(root);

        let mut btn = Node::new_element("button");
        btn.event_listeners
            .entry("click".into())
            .or_default()
            .push(EventListener { callback: noop });
        let btn_id = tree.insert(btn);

        let mut input = Node::new_element("div");
        input
            .event_listeners
            .entry("input".into())
            .or_default()
            .push(EventListener { callback: noop });
        let input_id = tree.insert(input);

        tree.append_child(root_id, btn_id);
        tree.append_child(root_id, input_id);

        let plan = build_render_plan(&tree, root_id).unwrap();
        assert!(!plan.has_click_handler);
        assert!(!plan.has_input_handler);
        assert_eq!(plan.children.len(), 2);
        assert!(plan.children[0].has_click_handler);
        assert!(!plan.children[0].has_input_handler);
        assert!(!plan.children[1].has_click_handler);
        assert!(plan.children[1].has_input_handler);
    }

    #[test]
    fn test_repaint_flag_with_render_plan_rebuild() {
        let mut tree = Tree::new();
        let root = Node::new_element("root");
        let root_id = tree.insert(root);

        let label = Node::new_element("span");
        let label_id = tree.insert(label);
        tree.append_child(root_id, label_id);

        let plan1 = build_render_plan(&tree, root_id).unwrap();
        assert_eq!(plan1.children.len(), 1);

        let label2 = Node::new_element("span");
        let label2_id = tree.insert(label2);
        tree.append_child(root_id, label2_id);

        let plan2 = build_render_plan(&tree, root_id).unwrap();
        assert_eq!(plan2.children.len(), 2);
    }

    #[test]
    fn test_html_semantic_tags_classify_correctly() {
        // Container elements -> Div
        for tag in &[
            "section", "header", "footer", "nav", "main", "article", "aside", "form", "fieldset",
        ] {
            assert_eq!(
                classify_tag(tag),
                GpuiElementKind::Div,
                "Expected {tag} to classify as Div"
            );
        }
        // Text-oriented elements -> TextContainer
        for tag in &[
            "a", "strong", "em", "code", "h1", "h2", "h3", "h4", "h5", "h6",
        ] {
            assert_eq!(
                classify_tag(tag),
                GpuiElementKind::TextContainer,
                "Expected {tag} to classify as TextContainer"
            );
        }
        // Block text tags -> TextContainer (GPUI maps these the same)
        for tag in &["pre"] {
            assert_eq!(
                classify_tag(tag),
                GpuiElementKind::TextContainer,
                "Expected {tag} to classify as TextContainer"
            );
        }
    }

    #[test]
    fn test_render_plan_tag_preserved() {
        let mut tree = Tree::new();
        let node = Node::new_element("button");
        let id = tree.insert(node);
        let plan = build_render_plan(&tree, id).unwrap();
        assert_eq!(plan.tag, "button");
        assert_eq!(plan.kind, GpuiElementKind::Div);
    }
}

/// Integration tests that exercise the full FFI -> shadow tree -> render plan pipeline.
#[cfg(test)]
#[allow(unused_unsafe)]
mod integration_tests {
    use super::*;
    use crate::{
        gpui_add_event_listener, gpui_append_child, gpui_create_element,
        gpui_create_text_node, gpui_destroy_element, gpui_dispatch_event,
        gpui_remove_child, gpui_render_plan_element_count, gpui_render_plan_json,
        gpui_free_string, gpui_reset_tree, gpui_set_attribute, gpui_set_style,
        gpui_set_text_content, gpui_verify_render_plan, lock_tree,
        GpuiElement as GpuiHandle,
    };
    use serial_test::serial;
    use std::ffi::{CStr, CString};
    use std::sync::atomic::{AtomicU32, Ordering};

    fn c(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    /// Helper: get the render plan as a parsed JSON value via the FFI function.
    fn get_plan_json(root: *mut GpuiHandle) -> serde_json::Value {
        unsafe {
            let json_ptr = gpui_render_plan_json(root);
            assert!(!json_ptr.is_null(), "render plan JSON should not be null");
            let json_str = CStr::from_ptr(json_ptr).to_str().unwrap().to_string();
            gpui_free_string(json_ptr);
            serde_json::from_str(&json_str).expect("render plan should be valid JSON")
        }
    }

    /// Helper: get the render plan directly (Tree API, not FFI).
    fn get_plan_direct(root: *mut GpuiHandle) -> RenderNode {
        let node_id = crate::tree::NodeId(unsafe { (*root).node_id });
        let tree = lock_tree();
        build_render_plan(&tree, node_id).expect("render plan should exist")
    }

    // ===================================================================
    // Full pipeline: counter app
    // ===================================================================

    #[test]
    #[serial]
    fn test_full_pipeline_counter_app() {
        unsafe {
            gpui_reset_tree();

            let root = gpui_create_element(c("div").as_ptr());
            let label = gpui_create_element(c("span").as_ptr());
            let text = gpui_create_text_node(c("Count: 0").as_ptr());
            gpui_append_child(label, text);
            gpui_append_child(root, label);

            let button = gpui_create_element(c("button").as_ptr());
            let btn_text = gpui_create_text_node(c("+").as_ptr());
            gpui_append_child(button, btn_text);

            extern "C" fn on_click() {}
            gpui_add_event_listener(button, c("click").as_ptr(), on_click);
            gpui_append_child(root, button);

            // Verify via direct render plan (Tree API)
            let plan = get_plan_direct(root);
            assert_eq!(plan.kind, GpuiElementKind::Div); // div -> Div
            assert_eq!(plan.children.len(), 2);

            // span -> TextContainer
            assert_eq!(plan.children[0].kind, GpuiElementKind::TextContainer);
            // text node -> TextNode with text
            assert_eq!(plan.children[0].children[0].kind, GpuiElementKind::TextNode);
            assert_eq!(plan.children[0].children[0].text.as_deref(), Some("Count: 0"));

            // button -> Div with click handler
            assert_eq!(plan.children[1].kind, GpuiElementKind::Div);
            assert!(plan.children[1].has_click_handler);
            assert_eq!(plan.children[1].children[0].text.as_deref(), Some("+"));

            // Verify via JSON FFI (the path Nim uses)
            let json = get_plan_json(root);
            assert_eq!(json["kind"], "Div");
            assert_eq!(json["children"].as_array().unwrap().len(), 2);
            assert_eq!(json["children"][0]["kind"], "TextContainer");
            assert_eq!(json["children"][0]["children"][0]["text"], "Count: 0");
            assert_eq!(json["children"][1]["kind"], "Div");
            assert_eq!(json["children"][1]["has_click_handler"], true);

            // Element count
            assert_eq!(gpui_render_plan_element_count(root), 5);
            assert_eq!(gpui_verify_render_plan(root), 1);

            gpui_destroy_element(root);
            gpui_destroy_element(label);
            gpui_destroy_element(text);
            gpui_destroy_element(button);
            gpui_destroy_element(btn_text);
        }
    }

    // ===================================================================
    // Style propagation through render plan
    // ===================================================================

    #[test]
    #[serial]
    fn test_render_plan_styles_via_ffi() {
        unsafe {
            gpui_reset_tree();

            let div = gpui_create_element(c("div").as_ptr());
            gpui_set_style(div, c("background-color").as_ptr(), c("red").as_ptr());
            gpui_set_style(div, c("width").as_ptr(), c("200px").as_ptr());
            gpui_set_style(div, c("flex-direction").as_ptr(), c("row").as_ptr());
            gpui_set_style(div, c("font-size").as_ptr(), c("16").as_ptr());
            gpui_set_style(div, c("border-radius").as_ptr(), c("8").as_ptr());
            gpui_set_style(div, c("padding").as_ptr(), c("10").as_ptr());

            let plan = get_plan_direct(div);
            assert_eq!(plan.styles.bg.as_deref(), Some("red"));
            assert_eq!(plan.styles.w.as_deref(), Some("200px"));
            assert_eq!(plan.styles.flex_direction.as_deref(), Some("row"));
            assert_eq!(plan.styles.text_size.as_deref(), Some("16"));
            assert_eq!(plan.styles.rounded.as_deref(), Some("8"));
            assert_eq!(plan.styles.p.as_deref(), Some("10"));

            // Also verify via JSON
            let json = get_plan_json(div);
            assert_eq!(json["styles"]["bg"], "red");
            assert_eq!(json["styles"]["w"], "200px");
            assert_eq!(json["styles"]["flex_direction"], "row");

            gpui_destroy_element(div);
        }
    }

    #[test]
    #[serial]
    fn test_render_plan_styles_column_direction() {
        unsafe {
            gpui_reset_tree();
            let div = gpui_create_element(c("div").as_ptr());
            gpui_set_style(div, c("flex-direction").as_ptr(), c("column").as_ptr());
            let plan = get_plan_direct(div);
            assert_eq!(plan.styles.flex_direction.as_deref(), Some("column"));
            gpui_destroy_element(div);
        }
    }

    #[test]
    #[serial]
    fn test_render_plan_styles_from_attributes_fallback() {
        unsafe {
            gpui_reset_tree();
            let div = gpui_create_element(c("div").as_ptr());
            gpui_set_attribute(div, c("width").as_ptr(), c("300").as_ptr());
            gpui_set_attribute(div, c("height").as_ptr(), c("200").as_ptr());
            let plan = get_plan_direct(div);
            assert_eq!(plan.styles.w.as_deref(), Some("300"));
            assert_eq!(plan.styles.h.as_deref(), Some("200"));
            gpui_destroy_element(div);
        }
    }

    // ===================================================================
    // Event handler wiring through render plan
    // ===================================================================

    #[test]
    #[serial]
    fn test_render_plan_event_handlers_via_ffi() {
        unsafe {
            gpui_reset_tree();

            static CLICKED: AtomicU32 = AtomicU32::new(0);
            extern "C" fn on_click() {
                CLICKED.fetch_add(1, Ordering::SeqCst);
            }
            CLICKED.store(0, Ordering::SeqCst);

            let button = gpui_create_element(c("button").as_ptr());
            gpui_add_event_listener(button, c("click").as_ptr(), on_click);

            let plan = get_plan_direct(button);
            assert!(plan.has_click_handler);
            assert!(!plan.has_input_handler);

            // Dispatch through the shadow tree -- verify callback fires
            gpui_dispatch_event(button, c("click").as_ptr());
            assert_eq!(CLICKED.load(Ordering::SeqCst), 1);

            gpui_destroy_element(button);
        }
    }

    #[test]
    #[serial]
    fn test_render_plan_multiple_event_types_via_ffi() {
        unsafe {
            gpui_reset_tree();
            extern "C" fn noop() {}

            let node = gpui_create_element(c("div").as_ptr());
            gpui_add_event_listener(node, c("click").as_ptr(), noop);
            gpui_add_event_listener(node, c("input").as_ptr(), noop);
            gpui_add_event_listener(node, c("hover").as_ptr(), noop);

            let plan = get_plan_direct(node);
            assert!(plan.has_click_handler);
            assert!(plan.has_input_handler);
            assert_eq!(plan.event_names.len(), 3);
            assert!(plan.event_names.contains(&"click".to_string()));
            assert!(plan.event_names.contains(&"input".to_string()));
            assert!(plan.event_names.contains(&"hover".to_string()));

            gpui_destroy_element(node);
        }
    }

    #[test]
    #[serial]
    fn test_render_plan_no_handlers_by_default_via_ffi() {
        unsafe {
            gpui_reset_tree();
            let div = gpui_create_element(c("div").as_ptr());
            let plan = get_plan_direct(div);
            assert!(!plan.has_click_handler);
            assert!(!plan.has_input_handler);
            assert!(plan.event_names.is_empty());
            gpui_destroy_element(div);
        }
    }

    // ===================================================================
    // Full demo app render plan (task manager)
    // ===================================================================

    #[test]
    #[serial]
    fn test_full_demo_app_render_plan() {
        unsafe {
            gpui_reset_tree();

            let app = gpui_create_element(c("div").as_ptr());

            // Header
            let header = gpui_create_element(c("header").as_ptr());
            let title = gpui_create_element(c("h1").as_ptr());
            let title_text = gpui_create_text_node(c("Task Manager").as_ptr());
            gpui_append_child(title, title_text);
            gpui_append_child(header, title);
            gpui_append_child(app, header);

            // Input area
            let input_area = gpui_create_element(c("div").as_ptr());
            let input = gpui_create_element(c("input").as_ptr());
            let add_btn = gpui_create_element(c("button").as_ptr());
            let add_text = gpui_create_text_node(c("Add").as_ptr());
            extern "C" fn on_add() {}
            gpui_add_event_listener(add_btn, c("click").as_ptr(), on_add);
            gpui_append_child(add_btn, add_text);
            gpui_append_child(input_area, input);
            gpui_append_child(input_area, add_btn);
            gpui_append_child(app, input_area);

            // Task list
            let task_list = gpui_create_element(c("ul").as_ptr());
            let mut task_handles = Vec::new();
            for task_name in &["Design API", "Write tests"] {
                let li = gpui_create_element(c("li").as_ptr());
                let span = gpui_create_element(c("span").as_ptr());
                let text = gpui_create_text_node(c(task_name).as_ptr());
                gpui_append_child(span, text);
                gpui_append_child(li, span);
                gpui_append_child(task_list, li);
                task_handles.push((li, span, text));
            }
            gpui_append_child(app, task_list);

            // Footer
            let footer = gpui_create_element(c("footer").as_ptr());
            let count_span = gpui_create_element(c("span").as_ptr());
            let count_text = gpui_create_text_node(c("2 tasks").as_ptr());
            gpui_append_child(count_span, count_text);
            gpui_append_child(footer, count_span);
            gpui_append_child(app, footer);

            // Verify render plan
            let plan = get_plan_direct(app);

            assert_eq!(plan.kind, GpuiElementKind::Div);
            assert_eq!(plan.children.len(), 4);

            // header -> Div
            assert_eq!(plan.children[0].kind, GpuiElementKind::Div);
            // h1 -> TextContainer
            assert_eq!(plan.children[0].children[0].kind, GpuiElementKind::TextContainer);

            // input area -> Div with 2 children
            assert_eq!(plan.children[1].kind, GpuiElementKind::Div);
            assert_eq!(plan.children[1].children.len(), 2);
            // add button -> Div with click handler
            assert!(plan.children[1].children[1].has_click_handler);

            // task list -> Div with 2 children
            assert_eq!(plan.children[2].kind, GpuiElementKind::Div);
            assert_eq!(plan.children[2].children.len(), 2);

            // First task: li -> Div > span -> TextContainer > text
            assert_eq!(plan.children[2].children[0].kind, GpuiElementKind::Div);
            assert_eq!(
                plan.children[2].children[0].children[0].kind,
                GpuiElementKind::TextContainer
            );
            assert_eq!(
                plan.children[2].children[0].children[0].children[0]
                    .text
                    .as_deref(),
                Some("Design API")
            );

            // Second task
            assert_eq!(
                plan.children[2].children[1].children[0].children[0]
                    .text
                    .as_deref(),
                Some("Write tests")
            );

            // footer -> Div > span -> TextContainer
            assert_eq!(plan.children[3].kind, GpuiElementKind::Div);
            assert_eq!(
                plan.children[3].children[0].kind,
                GpuiElementKind::TextContainer
            );
            assert_eq!(
                plan.children[3].children[0].children[0].text.as_deref(),
                Some("2 tasks")
            );

            // Total element count
            assert_eq!(gpui_render_plan_element_count(app), 18);

            // Clean up
            gpui_destroy_element(app);
            gpui_destroy_element(header);
            gpui_destroy_element(title);
            gpui_destroy_element(title_text);
            gpui_destroy_element(input_area);
            gpui_destroy_element(input);
            gpui_destroy_element(add_btn);
            gpui_destroy_element(add_text);
            gpui_destroy_element(task_list);
            gpui_destroy_element(footer);
            gpui_destroy_element(count_span);
            gpui_destroy_element(count_text);
            for (li, span, text) in task_handles {
                gpui_destroy_element(li);
                gpui_destroy_element(span);
                gpui_destroy_element(text);
            }
        }
    }

    // ===================================================================
    // Render plan after mutations
    // ===================================================================

    #[test]
    #[serial]
    fn test_render_plan_after_adding_child() {
        unsafe {
            gpui_reset_tree();
            let root = gpui_create_element(c("div").as_ptr());
            let label1 = gpui_create_element(c("span").as_ptr());
            gpui_append_child(root, label1);

            let plan1 = get_plan_direct(root);
            assert_eq!(plan1.children.len(), 1);
            assert_eq!(gpui_render_plan_element_count(root), 2);

            let label2 = gpui_create_element(c("span").as_ptr());
            gpui_append_child(root, label2);

            let plan2 = get_plan_direct(root);
            assert_eq!(plan2.children.len(), 2);
            assert_eq!(gpui_render_plan_element_count(root), 3);

            gpui_destroy_element(root);
            gpui_destroy_element(label1);
            gpui_destroy_element(label2);
        }
    }

    #[test]
    #[serial]
    fn test_render_plan_after_removing_child() {
        unsafe {
            gpui_reset_tree();
            let root = gpui_create_element(c("div").as_ptr());
            let c1 = gpui_create_element(c("span").as_ptr());
            let c2 = gpui_create_element(c("span").as_ptr());
            gpui_append_child(root, c1);
            gpui_append_child(root, c2);
            assert_eq!(gpui_render_plan_element_count(root), 3);

            gpui_remove_child(root, c1);
            let plan = get_plan_direct(root);
            assert_eq!(plan.children.len(), 1);
            assert_eq!(gpui_render_plan_element_count(root), 2);

            gpui_destroy_element(root);
            gpui_destroy_element(c1);
            gpui_destroy_element(c2);
        }
    }

    #[test]
    #[serial]
    fn test_render_plan_after_text_update() {
        unsafe {
            gpui_reset_tree();
            let root = gpui_create_element(c("div").as_ptr());
            let text_node = gpui_create_text_node(c("initial").as_ptr());
            gpui_append_child(root, text_node);

            let plan1 = get_plan_direct(root);
            assert_eq!(plan1.children[0].text.as_deref(), Some("initial"));

            gpui_set_text_content(text_node, c("updated").as_ptr());

            let plan2 = get_plan_direct(root);
            assert_eq!(plan2.children[0].text.as_deref(), Some("updated"));

            gpui_destroy_element(root);
            gpui_destroy_element(text_node);
        }
    }

    #[test]
    #[serial]
    fn test_render_plan_after_style_change() {
        unsafe {
            gpui_reset_tree();
            let div = gpui_create_element(c("div").as_ptr());
            gpui_set_style(div, c("background").as_ptr(), c("red").as_ptr());

            let plan1 = get_plan_direct(div);
            assert_eq!(plan1.styles.bg.as_deref(), Some("red"));

            gpui_set_style(div, c("background").as_ptr(), c("blue").as_ptr());

            let plan2 = get_plan_direct(div);
            assert_eq!(plan2.styles.bg.as_deref(), Some("blue"));

            gpui_destroy_element(div);
        }
    }

    #[test]
    #[serial]
    fn test_render_plan_after_adding_event_handler() {
        unsafe {
            gpui_reset_tree();
            let div = gpui_create_element(c("div").as_ptr());

            let plan1 = get_plan_direct(div);
            assert!(!plan1.has_click_handler);

            extern "C" fn noop() {}
            gpui_add_event_listener(div, c("click").as_ptr(), noop);

            let plan2 = get_plan_direct(div);
            assert!(plan2.has_click_handler);

            gpui_destroy_element(div);
        }
    }

    // ===================================================================
    // Tag classification through FFI -> render plan
    // ===================================================================

    #[test]
    #[serial]
    fn test_tag_classification_through_ffi() {
        let test_cases: Vec<(&str, GpuiElementKind)> = vec![
            ("div", GpuiElementKind::Div),
            ("section", GpuiElementKind::Div),
            ("header", GpuiElementKind::Div),
            ("footer", GpuiElementKind::Div),
            ("button", GpuiElementKind::Div),
            ("span", GpuiElementKind::TextContainer),
            ("label", GpuiElementKind::TextContainer),
            ("h1", GpuiElementKind::TextContainer),
            ("a", GpuiElementKind::TextContainer),
            ("strong", GpuiElementKind::TextContainer),
            ("p", GpuiElementKind::TextContainer),
            ("pre", GpuiElementKind::TextContainer),
            ("img", GpuiElementKind::Img),
            ("svg", GpuiElementKind::Svg),
        ];

        for (tag, expected_kind) in test_cases {
            unsafe {
                gpui_reset_tree();
                let elem = gpui_create_element(c(tag).as_ptr());
                let plan = get_plan_direct(elem);
                assert_eq!(
                    plan.kind, expected_kind,
                    "Tag '{}' should classify as {:?}",
                    tag, expected_kind
                );
                gpui_destroy_element(elem);
            }
        }
    }

    #[test]
    #[serial]
    fn test_text_node_classifies_as_text_node_via_ffi() {
        unsafe {
            gpui_reset_tree();
            let text = gpui_create_text_node(c("hello").as_ptr());
            let plan = get_plan_direct(text);
            assert_eq!(plan.kind, GpuiElementKind::TextNode);
            assert_eq!(plan.text.as_deref(), Some("hello"));
            gpui_destroy_element(text);
        }
    }

    // ===================================================================
    // Null / edge cases
    // ===================================================================

    #[test]
    #[serial]
    fn test_render_plan_null_handle() {
        unsafe {
            let json = gpui_render_plan_json(std::ptr::null_mut());
            assert!(json.is_null());
            assert_eq!(gpui_render_plan_element_count(std::ptr::null_mut()), 0);
            assert_eq!(gpui_verify_render_plan(std::ptr::null_mut()), 0);
        }
    }

    // ===================================================================
    // Child handlers preserved in plan
    // ===================================================================

    #[test]
    #[serial]
    fn test_render_plan_child_handlers_preserved() {
        unsafe {
            gpui_reset_tree();
            extern "C" fn on_click() {}
            extern "C" fn on_input() {}

            let root = gpui_create_element(c("div").as_ptr());
            let btn = gpui_create_element(c("button").as_ptr());
            gpui_add_event_listener(btn, c("click").as_ptr(), on_click);
            let input_div = gpui_create_element(c("div").as_ptr());
            gpui_add_event_listener(input_div, c("input").as_ptr(), on_input);

            gpui_append_child(root, btn);
            gpui_append_child(root, input_div);

            let plan = get_plan_direct(root);
            assert!(!plan.has_click_handler);
            assert!(!plan.has_input_handler);
            assert!(plan.children[0].has_click_handler);
            assert!(!plan.children[0].has_input_handler);
            assert!(!plan.children[1].has_click_handler);
            assert!(plan.children[1].has_input_handler);

            gpui_destroy_element(root);
            gpui_destroy_element(btn);
            gpui_destroy_element(input_div);
        }
    }

    // ===================================================================
    // Deep tree render plan
    // ===================================================================

    #[test]
    #[serial]
    fn test_render_plan_deep_nested_tree_via_ffi() {
        unsafe {
            gpui_reset_tree();
            let root = gpui_create_element(c("root").as_ptr());
            let div = gpui_create_element(c("div").as_ptr());
            let span = gpui_create_element(c("span").as_ptr());
            let text = gpui_create_text_node(c("deep").as_ptr());

            gpui_append_child(span, text);
            gpui_append_child(div, span);
            gpui_append_child(root, div);

            let plan = get_plan_direct(root);
            assert_eq!(count_render_nodes(&plan), 4);
            assert_eq!(
                plan.children[0].children[0].children[0].text.as_deref(),
                Some("deep")
            );

            gpui_destroy_element(root);
            gpui_destroy_element(div);
            gpui_destroy_element(span);
            gpui_destroy_element(text);
        }
    }

    // ===================================================================
    // Event dispatch + plan rebuild (simulates reactive update)
    // ===================================================================

    #[test]
    #[serial]
    fn test_event_dispatch_then_rebuild_plan() {
        unsafe {
            gpui_reset_tree();

            static COUNTER: AtomicU32 = AtomicU32::new(0);
            COUNTER.store(0, Ordering::SeqCst);
            extern "C" fn increment() {
                COUNTER.fetch_add(1, Ordering::SeqCst);
            }

            let root = gpui_create_element(c("div").as_ptr());
            let btn = gpui_create_element(c("button").as_ptr());
            let label = gpui_create_text_node(c("Count: 0").as_ptr());
            gpui_add_event_listener(btn, c("click").as_ptr(), increment);
            gpui_append_child(root, label);
            gpui_append_child(root, btn);

            // Initial plan
            let plan1 = get_plan_direct(root);
            assert_eq!(plan1.children[0].text.as_deref(), Some("Count: 0"));

            // Simulate click
            gpui_dispatch_event(btn, c("click").as_ptr());
            assert_eq!(COUNTER.load(Ordering::SeqCst), 1);

            // Simulate reactive update
            gpui_set_text_content(label, c("Count: 1").as_ptr());

            // Rebuild plan -- should reflect the mutation
            let plan2 = get_plan_direct(root);
            assert_eq!(plan2.children[0].text.as_deref(), Some("Count: 1"));

            gpui_destroy_element(root);
            gpui_destroy_element(btn);
            gpui_destroy_element(label);
        }
    }

    // ===================================================================
    // JSON serialization round-trip
    // ===================================================================

    #[test]
    #[serial]
    fn test_render_plan_json_round_trip() {
        unsafe {
            gpui_reset_tree();
            let div = gpui_create_element(c("div").as_ptr());
            gpui_set_style(div, c("background").as_ptr(), c("red").as_ptr());
            let span = gpui_create_element(c("span").as_ptr());
            let text = gpui_create_text_node(c("hello").as_ptr());
            gpui_append_child(span, text);
            gpui_append_child(div, span);

            extern "C" fn noop() {}
            gpui_add_event_listener(div, c("click").as_ptr(), noop);

            let json = get_plan_json(div);
            assert_eq!(json["kind"], "Div");
            assert_eq!(json["tag"], "div");
            assert_eq!(json["has_click_handler"], true);
            assert_eq!(json["styles"]["bg"], "red");
            assert_eq!(json["children"][0]["kind"], "TextContainer");
            assert_eq!(json["children"][0]["tag"], "span");
            assert_eq!(json["children"][0]["children"][0]["kind"], "TextNode");
            assert_eq!(json["children"][0]["children"][0]["text"], "hello");

            gpui_destroy_element(div);
            gpui_destroy_element(span);
            gpui_destroy_element(text);
        }
    }

    #[test]
    #[serial]
    fn test_render_plan_json_escaping() {
        unsafe {
            gpui_reset_tree();
            let node = gpui_create_text_node(c("hello \"world\"").as_ptr());
            let json = get_plan_json(node);
            assert_eq!(json["text"], "hello \"world\"");
            gpui_destroy_element(node);
        }
    }

    #[test]
    #[serial]
    fn test_render_plan_verify() {
        unsafe {
            gpui_reset_tree();
            let div = gpui_create_element(c("div").as_ptr());
            assert_eq!(gpui_verify_render_plan(div), 1);
            gpui_destroy_element(div);
        }
    }
}
