//! GPUI rendering integration tests.
//!
//! These tests verify that the shadow tree renders correctly through GPUI's
//! rendering pipeline using `TestAppContext` and `NimRootView`.
//!
//! Because the shadow tree and root node ID are global statics, all tests
//! must run serially.
//!
//! Note: We cannot use `#[gpui::test]` because the proc macro causes a
//! stack overflow when used from an external crate. Instead, we use
//! `gpui::run_test` and `TestAppContext::build` directly.

#![cfg(feature = "gpui-backend")]

// Import specific items from gpui rather than `use gpui::*` because gpui
// re-exports a `test` proc macro that would shadow `#[test]` and cause
// infinite recursion in the compiler.
use gpui::{div, AppContext, TestAppContext};
use gpui_nim_shim::gpui_app::{
    apply_styles_to_div, parse_color, parse_px, render_plan_to_gpui, NimRootView,
};
use gpui_nim_shim::render_sync::{build_render_plan, count_render_nodes, GpuiStyles, RenderNode};
use gpui_nim_shim::tree::{EventListener, GpuiElementKind, Node, NodeId};
use gpui_nim_shim::window;
use serial_test::serial;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Run a synchronous GPUI test with a `TestAppContext`.
fn run_gpui_test(name: &'static str, f: fn(&mut TestAppContext)) {
    gpui::run_test(
        1,
        &[],
        0,
        &mut |dispatcher, _seed| {
            let mut cx = TestAppContext::build(dispatcher.clone(), Some(name));
            f(&mut cx);
        },
        None,
    );
}

/// Reset the global shadow tree and root node ID to a clean state.
fn reset_global_tree() {
    gpui_nim_shim::gpui_reset_tree();
    window::reset_windows();
}

/// Populate the global shadow tree with a simple root -> child structure.
/// Returns the root NodeId.
fn setup_simple_tree(tag: &str, text: &str) -> NodeId {
    let mut tree = gpui_nim_shim::lock_tree();
    let mut root = Node::new_element("root");
    let root_id = root.id;

    let mut child = Node::new_element(tag);
    let child_id = child.id;
    child.set_text_content(text);
    root.children.push(child_id);
    child.parent = root_id;

    tree.insert(root);
    tree.insert(child);
    drop(tree);

    let mut root_ref = gpui_nim_shim::ROOT_NODE_ID
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    *root_ref = root_id;

    root_id
}

/// Populate a more complex tree: root with N div children, each containing text.
fn setup_multi_child_tree(children_text: &[&str]) -> NodeId {
    let mut tree = gpui_nim_shim::lock_tree();
    let mut root = Node::new_element("root");
    let root_id = root.id;

    for &text in children_text {
        let mut child = Node::new_element("div");
        let child_id = child.id;
        child.set_text_content(text);
        child.parent = root_id;
        root.children.push(child_id);
        tree.insert(child);
    }

    tree.insert(root);
    drop(tree);

    let mut root_ref = gpui_nim_shim::ROOT_NODE_ID
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    *root_ref = root_id;

    root_id
}

// ---------------------------------------------------------------------------
// Test 1: NimRootView renders in a test window without panicking
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_nim_root_view_renders_empty_tree() {
    run_gpui_test("test_nim_root_view_renders_empty_tree", |cx| {
        reset_global_tree();

        // With no root set, NimRootView should render the "No shadow tree root" fallback.
        let window = cx.add_window(|_window, _cx| NimRootView::new());
        cx.run_until_parked();

        let result = cx.read_window(&window, |_view, _app| "ok".to_string());
        assert!(result.is_ok());
    });
}

// ---------------------------------------------------------------------------
// Test 2: NimRootView renders a simple div with text content
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_nim_root_view_renders_div_with_text() {
    run_gpui_test("test_nim_root_view_renders_div_with_text", |cx| {
        reset_global_tree();
        setup_simple_tree("div", "Hello, GPUI!");

        let window = cx.add_window(|_window, _cx| NimRootView::new());
        cx.run_until_parked();

        let tree = gpui_nim_shim::lock_tree();
        let root_id = *gpui_nim_shim::ROOT_NODE_ID
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let plan = build_render_plan(&tree, root_id).expect("render plan should exist");
        assert_eq!(plan.tag, "root");
        assert_eq!(plan.children.len(), 1);
        assert_eq!(plan.children[0].tag, "div");
        assert_eq!(plan.children[0].text.as_deref(), Some("Hello, GPUI!"));
        drop(tree);

        let result = cx.read_window(&window, |_view, _app| true);
        assert!(result.is_ok());
    });
}

// ---------------------------------------------------------------------------
// Test 3: NimRootView renders multiple children
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_nim_root_view_renders_multiple_children() {
    run_gpui_test("test_nim_root_view_renders_multiple_children", |cx| {
        reset_global_tree();
        setup_multi_child_tree(&["Alpha", "Beta", "Gamma"]);

        let window = cx.add_window(|_window, _cx| NimRootView::new());
        cx.run_until_parked();

        let tree = gpui_nim_shim::lock_tree();
        let root_id = *gpui_nim_shim::ROOT_NODE_ID
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let plan = build_render_plan(&tree, root_id).expect("plan");
        assert_eq!(plan.children.len(), 3);
        assert_eq!(plan.children[0].text.as_deref(), Some("Alpha"));
        assert_eq!(plan.children[1].text.as_deref(), Some("Beta"));
        assert_eq!(plan.children[2].text.as_deref(), Some("Gamma"));
        assert_eq!(count_render_nodes(&plan), 4);
        drop(tree);

        let result = cx.read_window(&window, |_view, _app| true);
        assert!(result.is_ok());
    });
}

// ---------------------------------------------------------------------------
// Test 4: render_plan_to_gpui produces valid AnyElement for a text node
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_render_plan_to_gpui_text_node() {
    run_gpui_test("test_render_plan_to_gpui_text_node", |cx| {
        reset_global_tree();

        let _window = cx.add_window(|_window, _cx| {
            let plan = RenderNode {
                node_id: 999,
                kind: GpuiElementKind::TextNode,
                tag: "#text".to_string(),
                styles: GpuiStyles::default(),
                text: Some("Hello from text node".to_string()),
                has_click_handler: false,
                has_input_handler: false,
                event_names: vec![],
                children: vec![],
            };
            let _element = render_plan_to_gpui(&plan);
            NimRootView::new()
        });
        cx.run_until_parked();
    });
}

// ---------------------------------------------------------------------------
// Test 5: render_plan_to_gpui produces valid AnyElement for a styled div
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_render_plan_to_gpui_styled_div() {
    run_gpui_test("test_render_plan_to_gpui_styled_div", |cx| {
        reset_global_tree();

        let _window = cx.add_window(|_window, _cx| {
            let plan = RenderNode {
                node_id: 100,
                kind: GpuiElementKind::Div,
                tag: "div".to_string(),
                styles: GpuiStyles {
                    bg: Some("#ff0000".to_string()),
                    w: Some("200px".to_string()),
                    h: Some("100px".to_string()),
                    flex_direction: Some("row".to_string()),
                    p: Some("10px".to_string()),
                    gap: Some("5px".to_string()),
                    ..Default::default()
                },
                text: None,
                has_click_handler: false,
                has_input_handler: false,
                event_names: vec![],
                children: vec![RenderNode {
                    node_id: 101,
                    kind: GpuiElementKind::TextNode,
                    tag: "#text".to_string(),
                    styles: GpuiStyles::default(),
                    text: Some("Styled content".to_string()),
                    has_click_handler: false,
                    has_input_handler: false,
                    event_names: vec![],
                    children: vec![],
                }],
            };
            let _element = render_plan_to_gpui(&plan);
            NimRootView::new()
        });
        cx.run_until_parked();
    });
}

// ---------------------------------------------------------------------------
// Test 6: render_plan_to_gpui handles img placeholder
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_render_plan_to_gpui_img_placeholder() {
    run_gpui_test("test_render_plan_to_gpui_img_placeholder", |cx| {
        reset_global_tree();

        let _window = cx.add_window(|_window, _cx| {
            let plan = RenderNode {
                node_id: 200,
                kind: GpuiElementKind::Img,
                tag: "img".to_string(),
                styles: GpuiStyles::default(),
                text: Some("alt text".to_string()),
                has_click_handler: false,
                has_input_handler: false,
                event_names: vec![],
                children: vec![],
            };
            let _element = render_plan_to_gpui(&plan);
            NimRootView::new()
        });
        cx.run_until_parked();
    });
}

// ---------------------------------------------------------------------------
// Test 7: render_plan_to_gpui handles svg placeholder
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_render_plan_to_gpui_svg_placeholder() {
    run_gpui_test("test_render_plan_to_gpui_svg_placeholder", |cx| {
        reset_global_tree();

        let _window = cx.add_window(|_window, _cx| {
            let plan = RenderNode {
                node_id: 300,
                kind: GpuiElementKind::Svg,
                tag: "svg".to_string(),
                styles: GpuiStyles::default(),
                text: None,
                has_click_handler: false,
                has_input_handler: false,
                event_names: vec![],
                children: vec![],
            };
            let _element = render_plan_to_gpui(&plan);
            NimRootView::new()
        });
        cx.run_until_parked();
    });
}

// ---------------------------------------------------------------------------
// Test 8: render_plan_to_gpui handles TextContainer
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_render_plan_to_gpui_text_container() {
    run_gpui_test("test_render_plan_to_gpui_text_container", |cx| {
        reset_global_tree();

        let _window = cx.add_window(|_window, _cx| {
            let plan = RenderNode {
                node_id: 400,
                kind: GpuiElementKind::TextContainer,
                tag: "span".to_string(),
                styles: GpuiStyles {
                    text_color: Some("blue".to_string()),
                    ..Default::default()
                },
                text: Some("Span text".to_string()),
                has_click_handler: false,
                has_input_handler: false,
                event_names: vec![],
                children: vec![],
            };
            let _element = render_plan_to_gpui(&plan);
            NimRootView::new()
        });
        cx.run_until_parked();
    });
}

// ---------------------------------------------------------------------------
// Test 9: Full pipeline: shadow tree -> render plan -> GPUI elements in window
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_full_pipeline_shadow_tree_to_gpui() {
    run_gpui_test("test_full_pipeline_shadow_tree_to_gpui", |cx| {
        reset_global_tree();

        // Build a realistic shadow tree: root -> column layout with header + content
        {
            let mut tree = gpui_nim_shim::lock_tree();

            let mut root = Node::new_element("root");
            let root_id = root.id;

            let mut header = Node::new_element("h1");
            let header_id = header.id;
            header.set_text_content("My App");
            header.parent = root_id;

            let mut content = Node::new_element("div");
            let content_id = content.id;
            content
                .styles
                .insert("flex-direction".to_string(), "column".to_string());
            content
                .styles
                .insert("padding".to_string(), "16px".to_string());
            content.parent = root_id;

            let mut paragraph = Node::new_element("p");
            let para_id = paragraph.id;
            paragraph.set_text_content("Welcome to the app!");
            paragraph.parent = content_id;

            content.children.push(para_id);
            root.children.push(header_id);
            root.children.push(content_id);

            tree.insert(root);
            tree.insert(header);
            tree.insert(content);
            tree.insert(paragraph);

            drop(tree);
            let mut root_ref = gpui_nim_shim::ROOT_NODE_ID
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *root_ref = root_id;
        }

        let window = cx.add_window(|_window, _cx| NimRootView::new());
        cx.run_until_parked();

        let tree = gpui_nim_shim::lock_tree();
        let root_id = *gpui_nim_shim::ROOT_NODE_ID
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let plan = build_render_plan(&tree, root_id).expect("plan");

        assert_eq!(plan.tag, "root");
        assert_eq!(plan.children.len(), 2);
        assert_eq!(plan.children[0].tag, "h1");
        assert_eq!(plan.children[0].kind, GpuiElementKind::TextContainer);
        assert_eq!(plan.children[0].text.as_deref(), Some("My App"));
        assert_eq!(plan.children[1].tag, "div");
        assert_eq!(
            plan.children[1].styles.flex_direction.as_deref(),
            Some("column")
        );
        assert_eq!(plan.children[1].styles.p.as_deref(), Some("16px"));
        assert_eq!(plan.children[1].children.len(), 1);
        assert_eq!(plan.children[1].children[0].tag, "p");
        assert_eq!(
            plan.children[1].children[0].text.as_deref(),
            Some("Welcome to the app!")
        );
        assert_eq!(count_render_nodes(&plan), 4);
        drop(tree);

        let result = cx.read_window(&window, |_view, _app| true);
        assert!(result.is_ok());
    });
}

// ---------------------------------------------------------------------------
// Test 10: Nested div tree renders without stack overflow
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_deeply_nested_tree_renders() {
    run_gpui_test("test_deeply_nested_tree_renders", |cx| {
        reset_global_tree();

        let root_id;
        {
            let mut tree = gpui_nim_shim::lock_tree();
            let mut root = Node::new_element("root");
            root_id = root.id;
            let mut parent_id = root_id;

            // Create 20 levels of nesting.
            let mut nodes_to_insert: Vec<(NodeId, NodeId, Node)> = vec![];
            for i in 0..20 {
                let mut child = Node::new_element("div");
                let child_id = child.id;
                child.parent = parent_id;

                if i == 19 {
                    child.set_text_content("Deep leaf");
                }

                nodes_to_insert.push((parent_id, child_id, child));
                parent_id = child_id;
            }

            // Wire up root's first child.
            if let Some((_, cid, _)) = nodes_to_insert.first() {
                root.children.push(*cid);
            }
            tree.insert(root);

            // Wire intermediate parent->child links and insert.
            for i in 0..nodes_to_insert.len() {
                if i + 1 < nodes_to_insert.len() {
                    let next_cid = nodes_to_insert[i + 1].1;
                    nodes_to_insert[i].2.children.push(next_cid);
                }
            }
            for (_, _, node) in nodes_to_insert {
                tree.insert(node);
            }

            drop(tree);
            let mut root_ref = gpui_nim_shim::ROOT_NODE_ID
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *root_ref = root_id;
        }

        let window = cx.add_window(|_window, _cx| NimRootView::new());
        cx.run_until_parked();

        let tree = gpui_nim_shim::lock_tree();
        let plan = build_render_plan(&tree, root_id).expect("plan");
        assert_eq!(count_render_nodes(&plan), 21); // 1 root + 20 nested divs
        drop(tree);

        let result = cx.read_window(&window, |_view, _app| true);
        assert!(result.is_ok());
    });
}

// ---------------------------------------------------------------------------
// Test 11: parse_color helper
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_parse_color_hex_and_named() {
    run_gpui_test("test_parse_color_hex_and_named", |_cx| {
        // Hex colors
        assert!(parse_color("#ff0000").is_some());
        assert!(parse_color("#00ff00").is_some());
        assert!(parse_color("#0000ff").is_some());

        // Named colors
        assert!(parse_color("red").is_some());
        assert!(parse_color("green").is_some());
        assert!(parse_color("blue").is_some());
        assert!(parse_color("white").is_some());
        assert!(parse_color("black").is_some());
        assert!(parse_color("gray").is_some());

        // Invalid
        assert!(parse_color("notacolor").is_none());
        assert!(parse_color("#xyz").is_none());
        assert!(parse_color("").is_none());
    });
}

// ---------------------------------------------------------------------------
// Test 12: parse_px helper
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_parse_px_values() {
    run_gpui_test("test_parse_px_values", |_cx| {
        assert_eq!(parse_px("100px"), Some(100.0));
        assert_eq!(parse_px("50"), Some(50.0));
        assert_eq!(parse_px("3.5px"), Some(3.5));
        assert_eq!(parse_px("  16px "), Some(16.0));
        assert!(parse_px("abc").is_none());
        assert!(parse_px("").is_none());
    });
}

// ---------------------------------------------------------------------------
// Test 13: apply_styles_to_div does not panic with all style fields populated
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_apply_styles_to_div_all_fields() {
    run_gpui_test("test_apply_styles_to_div_all_fields", |_cx| {
        let styles = GpuiStyles {
            bg: Some("#336699".to_string()),
            w: Some("200px".to_string()),
            h: Some("100px".to_string()),
            min_w: Some("50px".to_string()),
            min_h: Some("25px".to_string()),
            max_w: Some("400px".to_string()),
            max_h: Some("300px".to_string()),
            p: Some("10px".to_string()),
            m: Some("5px".to_string()),
            flex_direction: Some("row".to_string()),
            gap: Some("8px".to_string()),
            text_size: Some("14".to_string()),
            text_color: Some("white".to_string()),
            rounded: Some("4px".to_string()),
            items: Some("center".to_string()),
            justify: Some("space_between".to_string()),
            border_width: None,
            border_color: None,
            shadow: None,
            opacity: None,
            overflow: None,
            font_family: None,
            font_weight: None,
            font_style: None,
            line_height: None,
            letter_spacing: None,
            cursor: Some("pointer".to_string()),
            display: None,
            position: None,
        };
        let el = div();
        let _styled = apply_styles_to_div(el, &styles);
    });
}

// ---------------------------------------------------------------------------
// Test 14: apply_styles_to_div handles percentage widths
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_apply_styles_full_width_height() {
    run_gpui_test("test_apply_styles_full_width_height", |_cx| {
        let styles = GpuiStyles {
            w: Some("100%".to_string()),
            h: Some("full".to_string()),
            ..Default::default()
        };
        let el = div();
        let _styled = apply_styles_to_div(el, &styles);
    });
}

// ---------------------------------------------------------------------------
// Test 15: Shadow tree mutation triggers re-render without panic
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_shadow_tree_mutation_and_rerender() {
    run_gpui_test("test_shadow_tree_mutation_and_rerender", |cx| {
        reset_global_tree();
        let root_id = setup_simple_tree("p", "Initial text");

        let window = cx.add_window(|_window, _cx| NimRootView::new());
        cx.run_until_parked();

        // Mutate the shadow tree: change the text content.
        {
            let tree = gpui_nim_shim::lock_tree();
            let root_node = tree.get(root_id).expect("root exists");
            let child_id = root_node.children[0];
            drop(tree);

            let mut tree = gpui_nim_shim::lock_tree();
            let child = tree.get_mut(child_id).expect("child exists");
            child.set_text_content("Updated text");
            window::request_repaint();
        }

        cx.run_until_parked();

        // Verify the updated plan.
        let tree = gpui_nim_shim::lock_tree();
        let plan = build_render_plan(&tree, root_id).expect("plan");
        assert_eq!(plan.children[0].text.as_deref(), Some("Updated text"));
        drop(tree);

        let result = cx.read_window(&window, |_view, _app| true);
        assert!(result.is_ok());
    });
}

// ---------------------------------------------------------------------------
// Test 16: Button node with click handler renders correctly
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_button_with_click_handler_renders() {
    run_gpui_test("test_button_with_click_handler_renders", |cx| {
        reset_global_tree();

        let root_id;
        {
            let mut tree = gpui_nim_shim::lock_tree();
            let mut root = Node::new_element("root");
            root_id = root.id;

            let mut button = Node::new_element("button");
            let button_id = button.id;
            button.set_text_content("Click me");
            button.parent = root_id;

            extern "C" fn noop() {}
            button
                .event_listeners
                .entry("click".to_string())
                .or_default()
                .push(EventListener { callback: noop, callback_id: 0 });

            root.children.push(button_id);

            tree.insert(root);
            tree.insert(button);

            drop(tree);
            let mut root_ref = gpui_nim_shim::ROOT_NODE_ID
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *root_ref = root_id;
        }

        let window = cx.add_window(|_window, _cx| NimRootView::new());
        cx.run_until_parked();

        let tree = gpui_nim_shim::lock_tree();
        let plan = build_render_plan(&tree, root_id).expect("plan");
        assert_eq!(plan.children.len(), 1);
        assert!(plan.children[0].has_click_handler);
        assert_eq!(plan.children[0].text.as_deref(), Some("Click me"));
        drop(tree);

        let result = cx.read_window(&window, |_view, _app| true);
        assert!(result.is_ok());
    });
}

// ---------------------------------------------------------------------------
// Test 17: Mixed element types (div, span, img, svg) in one tree
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_mixed_element_types_render() {
    run_gpui_test("test_mixed_element_types_render", |cx| {
        reset_global_tree();

        let root_id;
        {
            let mut tree = gpui_nim_shim::lock_tree();
            let mut root = Node::new_element("root");
            root_id = root.id;

            let tags = vec![
                ("div", "A div"),
                ("span", "A span"),
                ("img", "alt-text"),
                ("svg", "icon"),
                ("h2", "Heading"),
                ("p", "Paragraph"),
            ];

            for (tag, text) in &tags {
                let mut child = Node::new_element(tag);
                let child_id = child.id;
                child.set_text_content(text);
                child.parent = root_id;
                root.children.push(child_id);
                tree.insert(child);
            }

            tree.insert(root);
            drop(tree);

            let mut root_ref = gpui_nim_shim::ROOT_NODE_ID
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *root_ref = root_id;
        }

        let window = cx.add_window(|_window, _cx| NimRootView::new());
        cx.run_until_parked();

        let tree = gpui_nim_shim::lock_tree();
        let plan = build_render_plan(&tree, root_id).expect("plan");

        assert_eq!(plan.children.len(), 6);
        assert_eq!(plan.children[0].kind, GpuiElementKind::Div);
        assert_eq!(plan.children[1].kind, GpuiElementKind::TextContainer); // span
        assert_eq!(plan.children[2].kind, GpuiElementKind::Img);
        assert_eq!(plan.children[3].kind, GpuiElementKind::Svg);
        assert_eq!(plan.children[4].kind, GpuiElementKind::TextContainer); // h2
        assert_eq!(plan.children[5].kind, GpuiElementKind::TextContainer); // p
        drop(tree);

        let result = cx.read_window(&window, |_view, _app| true);
        assert!(result.is_ok());
    });
}

// ---------------------------------------------------------------------------
// Test 18: Empty root element renders without panic
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn test_empty_root_element_renders() {
    run_gpui_test("test_empty_root_element_renders", |cx| {
        reset_global_tree();

        let root_id;
        {
            let mut tree = gpui_nim_shim::lock_tree();
            let root = Node::new_element("root");
            root_id = root.id;
            tree.insert(root);
            drop(tree);

            let mut root_ref = gpui_nim_shim::ROOT_NODE_ID
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *root_ref = root_id;
        }

        let window = cx.add_window(|_window, _cx| NimRootView::new());
        cx.run_until_parked();

        let tree = gpui_nim_shim::lock_tree();
        let plan = build_render_plan(&tree, root_id).expect("plan");
        assert_eq!(plan.children.len(), 0);
        assert_eq!(count_render_nodes(&plan), 1);
        drop(tree);

        let result = cx.read_window(&window, |_view, _app| true);
        assert!(result.is_ok());
    });
}
