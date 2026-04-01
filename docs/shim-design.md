# Rust Shim Design for GPUI Backend

This document describes the architecture of the gpui-nim-shim: how it bridges
IsoNim's imperative RendererBackend interface with GPUI's declarative element model.

---

## 1. Shadow Tree Architecture

### Why a Shadow Tree?

GPUI is declarative: UI is described by `Render::render()` returning an element tree
each frame. IsoNim's RendererBackend is imperative: it calls `createElement`,
`appendChild`, `setStyle`, etc. to build and mutate a tree over time.

The shadow tree bridges this gap. It is a simple tree of `Node` structs that:
- Stores the current state of the UI as manipulated by IsoNim
- Serves as input to a render-sync function that produces GPUI elements
- Lives entirely in Rust, behind a `Mutex<Tree>` for thread safety

This is the same architecture used in isonim-freya (Freya is also declarative).

### Node Structure

```
Node {
    id: NodeId          // unique identifier
    tag: String         // "div", "#text", "img", etc.
    text: String        // text content (for #text nodes and text elements)
    attributes: Vec<(String, String)>   // name-value pairs
    styles: Vec<(String, String)>       // CSS property-value pairs
    events: Vec<EventListener>          // registered event handlers
    children: Vec<NodeId>               // ordered child node IDs
    parent: Option<NodeId>              // back-pointer to parent
}
```

### Tree Operations

All 13 RendererBackend procs map to simple mutations on this structure:

- **createElement** -- allocates a Node, returns a handle
- **createTextNode** -- allocates a Node with tag="#text"
- **appendChild/insertBefore/removeChild** -- manipulate children/parent links
- **setAttribute/removeAttribute** -- manipulate the attributes vector
- **setTextContent** -- update the text field
- **setStyle** -- manipulate the styles vector
- **addEventListener** -- append to the events vector
- **firstChild/nextSibling/parentNode** -- traverse the tree

### Handle Management

The `GpuiElement` handle is a heap-allocated wrapper around a `NodeId`:

```rust
#[repr(C)]
pub struct GpuiElement { id: NodeId }
```

Handles are created with `Box::into_raw(Box::new(...))` and destroyed with
`Box::from_raw(...)`. The Nim side holds opaque `pointer` values. Multiple
handles can point to the same NodeId (the shadow tree node outlives any
individual handle).

---

## 2. Render-Sync: Shadow Tree to GPUI Elements

### Overview

When GPUI calls `Render::render()` on our root view, the render-sync function:

1. Locks the shadow tree
2. Walks it depth-first from the root
3. For each node, creates the appropriate GPUI element builder
4. Applies styles, attributes, and event handlers
5. Recursively adds children
6. Returns the completed element tree

### Pseudocode

```rust
fn render_node(tree: &Tree, node_id: NodeId, window: &mut Window, cx: &mut App) -> AnyElement {
    let node = tree.get(node_id);

    match node.tag.as_str() {
        "#text" => {
            // Text nodes become string children of their parent
            node.text.clone().into_any_element()
        }
        "img" => {
            let mut el = img();
            if let Some(src) = node.get_attr("src") {
                el = el.uri(src);
            }
            apply_styles(&mut el, &node.styles);
            el.into_any_element()
        }
        "svg" => {
            let mut el = svg();
            apply_styles(&mut el, &node.styles);
            el.into_any_element()
        }
        _ => {
            // Everything else is a div
            let mut el = div();
            apply_styles(&mut el, &node.styles);
            apply_events(&mut el, &node.events, window, cx);

            // Add children in order
            for &child_id in &node.children {
                el = el.child(render_node(tree, child_id, window, cx));
            }

            // If the node has direct text content, add it as a child
            if !node.text.is_empty() {
                el = el.child(node.text.clone());
            }

            el.into_any_element()
        }
    }
}
```

### Root View

The GPUI application has a single root view that holds a reference to the shadow tree:

```rust
struct NimRootView;

impl Render for NimRootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tree = lock_tree();
        if let Some(root_id) = tree.root() {
            render_node(&tree, root_id, window, &mut cx.to_app())
        } else {
            div() // empty
        }
    }
}
```

When Nim mutates the shadow tree, the shim calls `cx.notify()` on the root view
to trigger a re-render, which re-runs the render-sync.

---

## 3. Event Flow

### GPUI to Nim Event Path

```
User interaction (click, keypress)
  |
  v
GPUI dispatches event to element at hit position
  |
  v
Element's event handler (registered during render-sync) fires
  |
  v
Handler looks up the shadow tree node by NodeId
  |
  v
Finds matching EventListener(s) in node.events
  |
  v
Calls the C FFI callback function pointer
  |
  v
Nim trampoline dispatches to the registered Nim closure
  |
  v
Nim handler may call RendererBackend procs (mutating shadow tree)
  |
  v
Shadow tree mutation triggers cx.notify() -> re-render
```

### Event Type Mapping

| IsoNim Event Name | GPUI Handler Method |
|-------------------|---------------------|
| `"click"` | `.on_mouse_up(MouseButton::Left, ...)` |
| `"mousedown"` | `.on_mouse_down(MouseButton::Left, ...)` |
| `"mouseup"` | `.on_mouse_up(MouseButton::Left, ...)` |
| `"mousemove"` | `.on_mouse_move(...)` |
| `"keydown"` | `.on_key_down(...)` |
| `"keyup"` | `.on_key_up(...)` |
| `"scroll"` | `.on_scroll_wheel(...)` |
| `"focus"` | Focus tracking via `track_focus()` |
| `"blur"` | Focus tracking via `on_focus_out()` |

### Callback Mechanism

The current implementation uses a fixed trampoline pool (16 `extern "C" fn()` slots).
Each trampoline calls into the Nim callback registry by index.

For M2+, this should be replaced with a more scalable approach:
- A single `extern "C" fn(callback_id: u32)` trampoline
- A callback ID registry mapping IDs to Nim closures
- Event handlers pass the callback ID as captured state

### Event Data

Currently, event handlers are `extern "C" fn()` with no event data. For full
event support, we need to pass event information (mouse position, key code, etc.):

```rust
// Future API
type EventCallbackWithData = extern "C" fn(event_data: *const EventData);

#[repr(C)]
struct EventData {
    event_type: u32,        // enum discriminant
    mouse_x: f64,
    mouse_y: f64,
    key_code: u32,
    modifiers: u32,
}
```

---

## 4. Style Mapping

### CSS Property to GPUI Styled Method

The render-sync function parses CSS property/value strings and calls the
corresponding GPUI builder methods. This happens in a `apply_styles` function:

```rust
fn apply_styles(el: &mut Div, styles: &[(String, String)]) {
    for (prop, value) in styles {
        match prop.as_str() {
            "width" => { el = el.w(parse_dimension(value)); }
            "height" => { el = el.h(parse_dimension(value)); }
            "bg" | "background-color" => { el = el.bg(parse_color(value)); }
            "text_color" | "color" => { el = el.text_color(parse_color(value)); }
            "padding" => { el = el.p(parse_dimension(value)); }
            "margin" => { el = el.m(parse_dimension(value)); }
            "flex_direction" => match value.as_str() {
                "row" => { el = el.flex_row(); }
                "col" | "column" => { el = el.flex_col(); }
                _ => {}
            },
            "align_items" => match value.as_str() {
                "center" => { el = el.items_center(); }
                "start" => { el = el.items_start(); }
                "end" => { el = el.items_end(); }
                _ => {}
            },
            "justify_content" => match value.as_str() {
                "center" => { el = el.justify_center(); }
                "start" => { el = el.justify_start(); }
                "end" => { el = el.justify_end(); }
                "space_between" => { el = el.justify_between(); }
                _ => {}
            },
            "gap" => { el = el.gap(parse_dimension(value)); }
            "border_width" => { /* el = el.border_N() based on value */ }
            "border_color" => { el = el.border_color(parse_color(value)); }
            "corner_radius" => { el = el.rounded(parse_dimension(value)); }
            "shadow" => { el = el.shadow_lg(); }
            "opacity" => { /* el = el.opacity(parse_float(value)) */ }
            "cursor" => match value.as_str() {
                "pointer" => { el = el.cursor_pointer(); }
                _ => {}
            },
            _ => { /* unknown property -- ignore or log */ }
        }
    }
}
```

### Value Parsers

Required parsers for style values:

| Parser | Input Examples | Output |
|--------|---------------|--------|
| `parse_dimension` | "16px", "100%", "auto", "2rem" | `px(16.0)`, `relative(1.0)`, `auto()`, `rems(2.0)` |
| `parse_color` | "#ff0000", "rgb(255,0,0)", "red" | `rgb(0xff0000)` / `Hsla` |
| `parse_float` | "0.5", "1.0" | `f32` |
| `parse_font_weight` | "bold", "400", "normal" | `FontWeight` enum |

### Pre-Mapped Properties (Nim Side)

The Nim `renderer.nim` already maps CSS property names to GPUI-friendly names
before sending them to the Rust shim (e.g., "background-color" -> "bg",
"flex-direction" -> "flex_direction"). This reduces the parsing burden on the
Rust side.

---

## 5. Element Mapping

### HTML Tags to GPUI Elements

| HTML Tag | GPUI Element | Notes |
|----------|-------------|-------|
| `div`, `section`, `article`, `nav`, `header`, `footer` | `div()` | All containers are flexbox divs |
| `span`, `p`, `h1`-`h6`, `label`, `strong`, `em` | `div()` + text child | Text styling via `text_xl()`, `font_weight()`, etc. |
| `button` | `div()` + event handlers | Styled as clickable, with hover states |
| `input`, `textarea` | `div()` (placeholder) | Full input support requires gpui-component integration |
| `img` | `img()` | With `.uri(src)` for the image source |
| `svg` | `svg()` | With `.path(src)` for the SVG source |
| `ul`, `ol`, `li` | `div()` | Lists are flexbox columns with children |
| `form`, `fieldset` | `div()` | Containers with no special behavior |

### Text Handling

Text in GPUI is not an element -- it is a child of a div. The render-sync handles
`#text` shadow tree nodes by converting them to string children:

```rust
// Shadow tree: div > #text("Hello")
// GPUI: div().child("Hello")

// Shadow tree: div > span("Hello") + span(" World")
// GPUI: div().child(div().child("Hello")).child(div().child(" World"))
```

For styled text (different colors, weights within a paragraph), GPUI provides
`StyledText` with `TextRun`s. This maps to multiple text shadow nodes with
different style attributes.

---

## 6. Window and Application Lifecycle

### Launch Sequence

```
Nim calls gpui_launch(title, width, height, root_builder_callback)
  |
  v
Rust creates Application::new()
  |
  v
Application::run(|cx| {
    // Create root shadow tree node
    let root = create_root_node();

    // Call Nim's root_builder callback
    root_builder_callback(root_handle);
    // Nim builds the initial shadow tree via RendererBackend procs

    // Open GPUI window with NimRootView
    cx.open_window(WindowOptions { title, size, ... }, |_, cx| {
        cx.new(|_| NimRootView)
    });
})
```

### Update Cycle

```
1. Nim modifies shadow tree (via RendererBackend procs)
2. Shim calls cx.notify() on NimRootView
3. GPUI schedules a re-render
4. On next frame, GPUI calls NimRootView::render()
5. render() runs render-sync, walking the shadow tree
6. GPUI receives the element tree, runs layout (Taffy), paints (GPU)
```

### Batching

Multiple shadow tree mutations between frames are automatically batched --
`cx.notify()` only marks the view as dirty, and GPUI coalesces multiple
notifications into a single re-render.

### Shutdown

When the GPUI window closes, the application event loop exits, returning
control to the caller of `gpui_launch`. Cleanup:
- Shadow tree is cleared
- Callback registry is reset
- All GpuiElement handles become dangling (Nim should not use them after shutdown)

---

## 7. Implementation Phases

### M0 (Complete): Project Structure
- Shadow tree data structures
- 13 extern "C" functions for RendererBackend
- Nim bindings and renderer module
- Compile-time conformance checks

### M1 (Current): API Analysis
- This design document
- GPUI API analysis document
- Detailed comments in lib.rs

### M2 (Next): Render-Sync
- Implement `render_node()` function
- Implement `NimRootView` with `Render` trait
- Implement `apply_styles()` with value parsers
- Enable `gpui-backend` feature
- Basic window creation in `gpui_launch`

### M3: Event Integration
- Scalable callback mechanism (replace fixed trampolines)
- Event data passing (mouse position, key codes)
- Event type mapping (click, keydown, etc.)
- Focus management

### M4: Full Style Support
- Complete CSS property parser
- Color parsing (hex, rgb, named colors)
- Dimension parsing (px, %, rem, auto)
- Hover/active state support

### M5: Polish and Testing
- Cross-renderer tests (compare GPUI output with DOM/Freya)
- Performance benchmarking
- Memory leak detection
- Platform-specific testing (macOS Metal, Linux Vulkan)
