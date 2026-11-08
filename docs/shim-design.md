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

### Only Drawn Properties Are Parsed

A CSS property is parsed into `GpuiStyles` and reported in the render plan's
`styles` object only if the shim applies it when it builds the drawn element.
The plan is read as a statement about the window, so a property it reports
but the window ignores lets a plan-level assertion pass on a wrong frame.

A property the shim cannot draw is not parsed: it falls through to the
unknown-property arm and is ignored like any other unsupported CSS.
`letter-spacing` is one — `gpui-pre` has no letter-spacing, and the shim
does not space glyphs itself. Adding a property to the parser and the dump
belongs in the same change that makes the element builder apply it.

### Text Align

`text-align` aligns a text element's lines in its box: `left` / `start`,
`center` and `right` / `end` (left-to-right text), applied to the element's
text style (`parse_text_align` in `gpui_app.rs`;
<https://www.w3.org/TR/css-text-3/#text-align-property>); `justify`, which
GPUI cannot draw, and anything else are ignored. Before 2026-11-08 it was not
parsed at all, so a centred notice's wrapped lines were drawn flush left. A
span inside an inline run fuses as before: with no width of its own, its
alignment draws nothing.

### Font Style

`font-style` is applied to the drawn element's text style: `italic` and
`oblique` (with or without an angle, which GPUI has no use for) ask for the
family's italic face, `normal` for its upright one, and any other value is
ignored, so the inherited style stays (`parse_font_style` in `gpui_app.rs`;
<https://www.w3.org/TR/css-fonts-4/#font-style-prop>). It was the one
exception to the rule above, parsed and reported but never drawn, until
2026-11-08.

GPUI **selects** a face; it does not **synthesise** one. Its Linux text
system (`gpui-pre-wgpu`, `cosmic_text_system.rs` `find_best_match`) scores
each face of the family by style and weight and takes the closest, and its
own test `font_properties_describe_the_selected_face` asserts that an
italic request on a family with no italic face returns the upright face.
So a family that ships an italic (Space Mono: `SpaceMono-Italic.ttf`) is
drawn italic, and one that does not (Space Grotesk, the design system's
interface face, ships Light to Bold upright only) is drawn **upright**. A
browser draws the second case differently: with `font-synthesis: style`
(the initial value) it slants the upright face. A consumer that must match
a browser's italic Space Grotesk therefore cannot get it from this shim
today; it would take a skewed glyph rasterisation in GPUI's text system.

`font-style` fuses in inline runs, like `font-weight`: a `HighlightStyle`
carries it per range (`inline_runs_fuse_spans_that_differ_only_in_style`).

### Font Variant Numeric

`font-variant-numeric` chooses the figures a text's digits take, and the shim
applies it as the OpenType features the CSS table names for each keyword
(<https://www.w3.org/TR/css-fonts-4/#font-variant-numeric-prop>):
`tabular-nums` is `tnum`, `proportional-nums` `pnum`, `lining-nums` `lnum`,
`oldstyle-nums` `onum`, `diagonal-fractions` `frac`, `stacked-fractions`
`afrc`, `ordinal` `ordn` and `slashed-zero` `zero`, each switched on in the
element's text style (`parse_font_variant_numeric` in `gpui_app.rs`).
`normal` switches all of them off. A value with an unknown word, a word
twice or two words of one group (`lining-nums oldstyle-nums`) is ignored, as
a browser ignores an invalid declaration, so the inherited features stay.

`gpui-pre` carries OpenType features per font (`TextStyle.font_features`,
`FontFeatures`), and its Linux text system hands them to cosmic-text's shaper
for each face it loads (`gpui-pre-wgpu` `cosmic_text_system.rs`,
`cosmic_font_features`), so a face without the feature draws as before.
GPUI's features REPLACE the inherited set instead of adding to it; no other
property of this shim sets features, so the two cannot disagree today.

Why it matters: a browser sets tabular digits one advance wide, and a
layout sized by them (a count, a recording button's number, a column of
ages) is narrower or wider than the same text in proportional figures. Space
Grotesk's proportional "1" is 416 units of its 1000-unit em and its "0" 641;
its `tnum` digits are all 620. Before 2026-11-08 the property was not parsed,
so CodeTracer's Agent Harbor recording buttons were 0.5-2 px off the
browser's and the active initiative tab cut its title a character early.
`test_tabular_nums_sets_every_digit_one_advance` (`tests/gpui_rendering.rs`)
shapes the digits with the platform's real text system and the committed
Space Grotesk fixture (`tests/fixtures/fonts/`, SIL OFL 1.1): every digit
must be 62 px at 100 px with the property, and the control without it keeps
the face's different advances. Without the shim applying the features the
test fails on the first digit (64.1 px).

A `HighlightStyle` has no font features, so a span that sets the property
does not fuse into an inline run (`fusable_child_text`); its neighbours
still fuse with each other
(`inline_chunks_keep_a_span_with_numeric_figures_own`). A CONTAINER that
sets it passes it to a fused run like any inherited text style.

### Dashed Borders

`border-style: dashed` draws the element's border dashed (GPUI's
`BorderStyle::Dashed`); any other value, or none, keeps the solid border. Only
`dashed` is parsed into a drawing; it is the one non-solid style a consumer
asks for (CodeTracer's tab-strip notices are outlined, not filled).

### Flex Basis

`flex-basis: Npx` sets a flex item's starting size on its line's main axis
(CSS Flexbox §7.3.3) apart from its `width`. A browser shrinks the items of an
overflowing line in proportion to their flex base size, which for an item
capped by `max-width` is its whole content, not the cap; with `flex-basis` set
to the content and `max-width` to the cap, GPUI's layout (Taffy) shrinks the
same way.

### Flex Shrink

`flex-shrink: N` (any non-negative number) is the item's shrink factor: an
overfull line is shrunk in proportion to factor times flex basis (CSS
Flexbox §9.7, <https://www.w3.org/TR/css-flexbox-1/#resolve-flexible-lengths>),
which Taffy implements. Until 2026-11-08 only `0` was applied and any other
factor was parsed and dropped, so an item meant to give way a hundred times
faster than its neighbour (CodeTracer's session-group counts, `flex: 0 100
auto`) shrank at the default 1. A negative or unparsable value is ignored.
`test_flex_shrink_factor_is_applied_to_the_layout` (`tests/gpui_rendering.rs`)
reads the laid-out widths back.

### Mask Image

`mask-image: url(<file>)` (also `-webkit-mask-image`) paints the element's
`background` colour THROUGH an SVG file's alpha instead of over the whole
box, which is how a web stylesheet draws a one-colour icon in a theme's ink
(`background-color: currentColor; mask-image: url(...)`;
<https://www.w3.org/TR/css-masking-1/#the-mask-image>). The builder adds an
absolutely positioned child that fills the box and, when painted,
rasterises the SVG at the box's DEVICE size (GPUI's own resvg rasteriser,
`SvgRenderer::render_parsed` with an exact size) into a picture of the
colour whose alpha is the mask's coverage (`mask_picture`), and paints that
picture one texel per device pixel (`Window::paint_image`, a polychrome
sprite; `with_mask_image` in `gpui_app.rs`). Pictures are cached per file,
device size and colour (64, evicted ones released like the `img` cache's).
What it does not do, and why none of it matters to its consumers: it masks
the background only, not the element's children or border; it fills the box
(`mask-size: contain`, `mask-position: center` when the mask's aspect is the
box's); `none`, a gradient or several layers leave the background unmasked.

Why not GPUI's `svg()`, which the shim used until 2026-11-08: it paints a
MONOCHROME sprite, and GPUI's monochrome-sprite shader is its glyph shader.
`fs_mono_sprite` (`gpui-pre-wgpu` `shaders.wgsl`) passes every coverage value
through `apply_contrast_and_gamma_correction`: the text system's light-on-dark
contrast enhancement (`ZED_FONTS_GRAYSCALE_ENHANCED_CONTRAST`, default 1.0,
scaled by how dark the ink is) and its gamma-1.8 alpha correction
(`ZED_FONTS_GAMMA`). A browser composites a mask at its linear coverage.
Measured 2026-11-08 on CodeTracer's "+" (a 1px cross on the half pixel,
`#b0b0b0` at 0.7 over a `#333333` disc): the browser draws the
half-covered arm pixels 94 and the three-quarter-covered centre 117, which
is `51 + 125 x 0.7 x 0.5` and `x 0.75`; `svg()` drew 105 and 121, which is
that shader's correction of 0.5 and 0.75 to 0.617 and 0.800 for a
brightness-0.69 ink, to the unit. It was neither the rasterisation scale
(`svg()` rasterises at twice the device size and its 2:1 bilinear
minification averages to the exact coverage on axis-aligned edges) nor
premultiplication. Changing the two environment variables would change every
glyph the window draws as well, so the mask leaves that pipeline instead.

Why not an `img` of a recoloured SVG either: GPUI rasterises an `img` SVG at
twice its intrinsic size (`SMOOTH_SVG_SCALE_FACTOR`), un-premultiplies it,
and on a 1x window minifies it with straight-alpha filtering, so the
transparent black around a shape darkens every antialiased edge (the same
"+" drew 64 and 93). The mask picture is rasterised at the device size and
sampled texel for texel, so there is no minification. The `img` path keeps
that defect for a picture that is not a mask; it is GPUI's, not this shim's.

`test_mask_picture_is_the_ink_at_linear_coverage` (`gpui_app.rs`) checks
the picture: the "+" at 12x12 has alpha 128 (+-1, resvg's rounding) on its
arms and 191 at its centre, where the corrected values would be 157 and
204. That the picture reaches the window as a polychrome sprite is shown by
the windows themselves (CodeTracer's CTF-P1b story captures), not by a test
here: no Linux test path in this repository reads a GPUI frame back short of
the headless-sway GUI lane.

### Laid-out Bounds

The shadow tree and the render plan say what an element is; where the window
put it is GPUI's layout, computed after the plan is handed over. An element
that carries the attribute `data-layout-probe` gets one more child, an
absolutely positioned full-size `canvas` that paints nothing and records the
element's padding box in window coordinates during prepaint (an absolute child
takes no part in the flex layout, so the probe does not move what it
measures). `gpui_layout_bounds(node, out)` (Nim: `renderer.layoutBounds`)
reads the latest frame's bounds back; `gpui_layout_bounds_reset` forgets
them. Without the GPUI backend nothing is laid out and every read answers 0.
The store is `layout_bounds.rs`, feature-less like `input` and
`frame_stats`, so the exported symbol set does not depend on the features.

### Inline-Run Fusion and Horizontal Insets

A container that declares `data-inline-text-runs` has its text-only
children drawn as one text element per run, so the run's width is rounded
once instead of once per span. The governing rule is the
`inlineTextRunsAttribute` doc comment in `src/isonim_gpui/renderer.nim`;
`fusable_child_text` in `render_sync.rs` implements it.

A `StyledText` highlight carries colour, weight and style but no padding, so a
span's padding used to keep that span out of every fused run. A span's own
`padding-left` / `padding-right` (a non-negative px length) now fuses when
the container declares `white-space: nowrap` or `pre` and no
`text-overflow`. Such a run is drawn by `InsetRunText` (`gpui_app.rs`): one
measured layout leaf that shapes the spans between insets as single lines,
places them at unrounded pen offsets separated by the device-pixel-snapped
insets, and lets GPUI round the total once. Runs without insets are still
drawn as a `StyledText`, unchanged. The one-line condition exists because
`InsetRunText` never wraps or truncates. A run whose text holds a hard line
break cannot be shaped as one line either, so it is split back at its padded
spans: those keep their own elements and the plain spans between them fuse
as a `StyledText`, exactly as before insets existed. Vertical padding, the `padding`
shorthand, margins and every other box style still keep a span as its own
element.

### Value Parsers

Required parsers for style values:

| Parser | Input Examples | Output |
|--------|---------------|--------|
| `parse_dimension` | "16px", "100%", "auto", "2rem" | `px(16.0)`, `relative(1.0)`, `auto()`, `rems(2.0)` |
| `parse_color` | "#ff0000", "rgb(255,0,0)", "red" | `rgb(0xff0000)` / `Hsla` |
| `parse_float` | "0.5", "1.0" | `f32` |
| `parse_font_weight` | "bold", "400", "normal" | `FontWeight` enum |
| `parse_font_style` | "italic", "oblique 10deg", "normal" | `FontStyle` (or `None`: inherit) |
| `parse_font_variant_numeric` | "tabular-nums", "slashed-zero lining-nums", "normal" | `FontFeatures` (or `None`: inherit) |

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
| `img`, `svg` with a `src` | flex `div()` holding `img(path)` | `file_image`: the picture fills a box that carries the element's own styles (size, `opacity`, margins, `flex-shrink`, ...); a raster file is decoded, anything else is rendered as SVG. See "Pictures" below |
| `img` with pixels (`gpui_set_image_rgba`) | flex `div()` holding `img(RenderImage)` | stretched to the element's box, which carries its styles. See "Pictures" below |
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

### Pictures

Both `img` arms draw the picture in a box (`picture_box` in `gpui_app.rs`)
that carries the element's own styles, and the picture fills the box
(`size_full`). The box is a **flex** container on purpose. Once a picture has
loaded, GPUI's `Img` states its intrinsic aspect ratio in its own layout
style (`gpui-pre` `elements/img.rs`, `request_layout`); in GPUI's default
block `div()`, Taffy then lets that ratio, not the box, decide the picture's
height, so a picture taller than its box was laid out taller than the size
the element states (a 10x16 SVG at 6x8 came out 6x9.5) and drew over its
neighbours. As a flex item the picture resolves `size_full` against the box
on both axes, which is what a browser does for an `<img>` with both `width`
and `height` stated. The pixel-buffer arm had the block box from its first
version; the file arm took it on 2026-11-08 and regressed CodeTracer's
flow-loop arrows and execution mark the same day, which is how it was found.

`test_a_non_square_picture_lands_at_its_declared_size`
(`tests/gpui_rendering.rs`, `cargo test --features gpui-backend`) reads the
picture's LAID-OUT bounds back from a `TestAppContext` window, for portrait
and landscape SVGs and a pixel buffer, after the picture has loaded. It
reads them through `picture_debug_selector`, a GPUI `debug_selector` on the
picture that records nothing unless GPUI is built with `test-support`. With
a block box it fails on the three cases whose picture is taller than its box.
A check of the box's style fields alone cannot see this: the box keeps the
stated size while the picture inside it does not.

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
