# GPUI API Analysis for IsoNim Integration

This document captures the results of analyzing GPUI's public API surface and mapping
it to IsoNim's RendererBackend interface. GPUI is Zed's hybrid immediate/retained-mode,
GPU-accelerated UI framework for Rust.

Crate: `gpui` (https://docs.rs/gpui/latest/gpui/)
Source: https://github.com/zed-industries/zed/tree/main/crates/gpui

---

## 1. Element Model

### Core Traits

**`Element`** -- the low-level trait for anything that participates in layout and painting.

```rust
pub trait Element: 'static + IntoElement {
    type RequestLayoutState;
    type PrepaintState;

    fn id(&self) -> Option<ElementId>;
    fn source_location(&self) -> Option<&'static Location<'static>>;
    fn request_layout(
        &mut self, id: Option<&GlobalElementId>,
        window: &mut Window, cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState);
    fn prepaint(
        &mut self, id: Option<&GlobalElementId>,
        bounds: Bounds<Pixels>, state: &mut Self::RequestLayoutState,
        window: &mut Window, cx: &mut App,
    ) -> Self::PrepaintState;
    fn paint(
        &mut self, id: Option<&GlobalElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window, cx: &mut App,
    );
}
```

Three-phase lifecycle per frame:
1. **request_layout** -- queries Taffy for flexbox layout, returns a LayoutId.
2. **prepaint** -- commits bounds, registers hitboxes.
3. **paint** -- draws quads, glyphs, images via GPU commands.

**`IntoElement`** -- conversion trait (like `Into<AnyElement>`). Any type that can
become a renderable element implements this. Strings, formatted text, and custom
components all implement it.

**`RenderOnce`** -- stateless component pattern, consumes `self`:

```rust
pub trait RenderOnce: 'static + Sized {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement;
}
```

Used with `#[derive(IntoElement)]` to create reusable UI building blocks that
don't maintain their own state across frames.

**`Render`** -- stateful view trait, borrows `&mut self`:

```rust
pub trait Render: 'static + Sized {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement;
}
```

Views implementing `Render` are re-rendered each frame when `cx.notify()` is called.
The `Context<Self>` parameter provides access to the entity's own state and the
application context.

### Element Builders

GPUI provides factory functions that return builder structs:

| Function | Returns | Purpose |
|----------|---------|---------|
| `div()` | `Div` | General-purpose flexbox container (the workhorse element) |
| `img()` | `Img` | Image element with caching |
| `svg()` | `Svg` | SVG rendering |
| `canvas()` | `Canvas` | Low-level custom painting |
| `text()` / `StyledText` | text element | Text rendering |

`Div` is the universal container. There is no separate "span", "button", or "section"
element -- everything is a `div()` with different styling and event handlers. Text
content is added as a child via `.child("some text")` or `.child(format!(...))`.

### ParentElement Trait

Elements that can contain children implement `ParentElement`:

```rust
trait ParentElement {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>);
    fn child(self, child: impl IntoElement) -> Self;       // single child
    fn children(self, children: impl IntoIterator<Item = impl IntoElement>) -> Self; // multiple
}
```

Children are added declaratively in the `render()` method via builder chaining.
There is **no imperative appendChild/removeChild** -- the element tree is rebuilt
each frame from the render function's return value.

### Practical Example: Hello World

```rust
use gpui::*;

struct HelloWorld { text: SharedString }

impl Render for HelloWorld {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .bg(rgb(0x505050))
            .size_full()
            .justify_center()
            .items_center()
            .text_xl()
            .text_color(rgb(0xffffff))
            .child(format!("Hello, {}!", &self.text))
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.open_window(WindowOptions::default(), |_, cx| {
            cx.new(|_| HelloWorld { text: "World".into() })
        }).unwrap();
    });
}
```

---

## 2. State Management

### Entity<T>

All application state is owned by the `App` (application context). User code accesses
state through `Entity<T>` handles -- reference-counted smart pointers with a type
parameter:

```rust
let counter: Entity<Counter> = cx.new(|_cx| Counter { count: 0 });
```

Entity handles are cheap to clone (like `Rc`). Actual data is stored in the App and
accessed through the handle + context:

```rust
// Reading
let count = counter.read(cx).count;

// Updating (leases the data to a callback)
counter.update(cx, |counter, cx| {
    counter.count += 1;
    cx.notify(); // signal that state changed, triggers re-render
});
```

### Context Types

| Type | Purpose |
|------|---------|
| `App` | Application-wide context: entity creation, globals, window management |
| `Window` | Window-specific: layout, painting, focus, event dispatch |
| `Context<T>` | Entity-specific: wraps App with entity data, provides `notify()` and `emit()` |

`Context<T>` derefs to `App`, so entity callbacks can access all application services.

### Observation and Reactivity

GPUI uses two complementary reactive systems:

1. **observe/notify** -- general state-change signals:
   ```rust
   cx.observe(&counter, |observer, observed, cx| {
       observer.count = observed.read(cx).count * 2;
   }).detach();
   ```

2. **subscribe/emit** -- typed event dispatch (see Events section below).

### Effect System

GPUI queues effects rather than executing them inline. When `cx.notify()` or
`cx.emit()` is called, the effect is pushed to a queue. At the end of each
`App::update()` call, the queue is flushed front-to-back. This prevents
reentrancy bugs that are common in traditional event emitter architectures.

---

## 3. Event System

### EventEmitter Trait

A marker trait associating an entity type with its emittable events:

```rust
pub trait EventEmitter<E: Any>: 'static { }
```

Events are emitted with `cx.emit(event)` from within an entity's `Context<T>`,
and subscribed to with `cx.subscribe(&entity, callback)`.

### Mouse and Keyboard Events

GPUI elements handle interaction via builder methods on `Div` (the `InteractiveElement` trait):

**Mouse events:**
- `on_mouse_down(button, handler)` / `on_mouse_up(button, handler)`
- `on_any_mouse_down(handler)`
- `on_mouse_move(handler)`
- `on_scroll_wheel(handler)`
- `on_mouse_down_out(handler)` -- click outside element

**Keyboard events:**
- `on_key_down(handler)` / `on_key_up(handler)`

**Convenience:**
- `on_click(handler)` -- not present on base Div, but available via gpui-component
- `hover(|style| style.bg(...))` -- hover state styling

Event handlers use `cx.listener()` to create properly-typed callbacks:

```rust
div()
    .on_mouse_up(MouseButton::Left, cx.listener(Self::on_click))
```

Where the handler signature is:

```rust
fn on_click(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
    self.count += 1;
    cx.notify();
}
```

### Action System

Actions are a higher-level abstraction for keyboard-driven commands:

```rust
actions!(counter, [Increment, Reset]);

// Bind keys
cx.bind_keys([
    KeyBinding::new("space", Increment, None),
    KeyBinding::new("r", Reset, None),
]);

// Handle in render
div()
    .track_focus(&self.focus_handle(cx))
    .on_action(cx.listener(Self::increment))
```

Actions propagate through the focus tree and can be conditionally available via
`KeyContext`.

### Counter Example (Events + State)

```rust
actions!(counter, [Increment, Reset]);

struct Counter {
    count: i32,
    focus_handle: FocusHandle,
}

impl Counter {
    fn new(cx: &mut Context<Self>) -> Self {
        Self { count: 0, focus_handle: cx.focus_handle() }
    }

    fn increment(&mut self, _: &Increment, _: &mut Window, cx: &mut Context<Self>) {
        self.count += 1;
        cx.notify();
    }

    fn on_increment_click(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.count += 1;
        cx.notify();
    }
}

impl Render for Counter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex().flex_col().gap_4()
            .bg(rgb(0x2e3440)).size(px(400.0))
            .justify_center().items_center()
            .text_xl().text_color(rgb(0xd8dee9))
            .track_focus(&self.focus_handle(cx))
            .on_action(cx.listener(Self::increment))
            .child(div().text_2xl().child(format!("Count: {}", self.count)))
            .child(
                div()
                    .bg(rgb(0x4c566a))
                    .hover(|s| s.bg(rgb(0x5e81ac)).cursor_pointer())
                    .rounded_lg().px_6().py_3()
                    .child("Increment")
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::on_increment_click))
            )
    }
}
```

---

## 4. Styling System

GPUI uses a **Tailwind-inspired** builder API. All styling is done via method chaining
on elements that implement the `Styled` trait. There are no CSS strings or stylesheets.

### Categories of Style Methods

**Layout (flexbox via Taffy):**
- `flex()`, `flex_row()`, `flex_col()`, `flex_wrap()`, `flex_none()`
- `flex_grow()`, `flex_shrink()`, `flex_basis()`
- `items_start()`, `items_center()`, `items_end()`, `items_baseline()`
- `justify_start()`, `justify_center()`, `justify_end()`, `justify_between()`, `justify_around()`
- `gap_0()` .. `gap_12()`, `gap_x()`, `gap_y()`
- `grid()`, `grid_cols()`, `grid_rows()`

**Sizing:**
- `w(px)`, `w_full()`, `w_auto()`, `w_1()` .. `w_128()`, `w_1_2()`, `w_1_3()`
- `h(px)`, `h_full()`, `h_auto()`, `h_1()` .. `h_128()`
- `size(px)`, `size_full()`
- `min_w()`, `max_w()`, `min_h()`, `max_h()`

**Spacing:**
- Padding: `p()`, `px()`, `py()`, `pt()`, `pb()`, `pl()`, `pr()`, `p_0()` .. `p_12()`
- Margin: `m()`, `mx()`, `my()`, `mt()`, `mb()`, `ml()`, `mr()`, `m_auto()`

**Colors:**
- `bg(color)` -- background
- `text_color(color)` -- text foreground
- `border_color(color)` -- border color
- Colors via `rgb(0xRRGGBB)`, `rgba(...)`, `Hsla` structs

**Typography:**
- `text_xs()` .. `text_3xl()` -- preset font sizes
- `font(name)`, `font_family(name)`, `font_weight(weight)`
- `italic()`, `line_height()`, `line_clamp()`

**Borders and Corners:**
- `border_0()` .. `border_32()` -- border width
- `border_t()`, `border_b()`, `border_l()`, `border_r()` -- directional
- `rounded()`, `rounded_sm()` .. `rounded_full()`, `rounded_lg()`

**Positioning:**
- `absolute()`, `relative()` (default)
- `top()`, `bottom()`, `left()`, `right()`, `inset()`

**Visual effects:**
- `shadow()`, `shadow_sm()`, `shadow_lg()`
- `opacity()`, `hidden()`, `invisible()`
- `cursor_pointer()`, `cursor_default()`, `cursor_text()`

**Hover states:**
- `hover(|style| style.bg(...).cursor_pointer())` -- closure-based hover styling

### Layout Engine

GPUI uses **Taffy** (a Rust implementation of CSS flexbox/grid) for layout.
`Window::request_layout()` creates Taffy layout nodes; `Window::compute_layout()`
runs the layout algorithm. Elements get their bounds from `layout_bounds(LayoutId)`.

---

## 5. RendererBackend Mapping Table

IsoNim's RendererBackend uses an imperative, DOM-like API. GPUI is purely
declarative -- there is no imperative tree manipulation API. The shadow tree
in the Rust shim bridges this gap.

| # | RendererBackend Proc | Shadow Tree Operation | GPUI Rendering Equivalent |
|---|---------------------|-----------------------|---------------------------|
| 1 | `createElement(tag)` | Allocate a Node in the shadow tree with the given tag | During render-sync: `div()`, `img()`, `svg()`, or `StyledText` depending on the tag |
| 2 | `createTextNode(text)` | Allocate a Node with tag="#text" and text content | During render-sync: passed as `.child(text)` to the parent element |
| 3 | `appendChild(parent, child)` | Set child.parent = parent; append child to parent.children | During render-sync: `.child(child_element)` in the parent's builder chain |
| 4 | `insertBefore(parent, child, ref)` | Insert child into parent.children before ref | During render-sync: children are emitted in order from the shadow tree |
| 5 | `removeChild(parent, child)` | Remove child from parent.children; clear child.parent | During render-sync: child is simply not included in the builder chain |
| 6 | `setAttribute(node, name, value)` | Store (name, value) in node.attributes | During render-sync: mapped to element builder methods (e.g. `.id()`, or stored as metadata) |
| 7 | `removeAttribute(node, name)` | Remove the named attribute from node.attributes | During render-sync: attribute absence means the builder method is not called |
| 8 | `setTextContent(node, text)` | Set node.text = text | During render-sync: `.child(text)` on the parent, or update `StyledText` content |
| 9 | `setStyle(node, prop, value)` | Store (prop, value) in node.styles | During render-sync: mapped to Styled trait methods (see style mapping below) |
| 10 | `addEventListener(node, event, cb)` | Store EventListener { event, callback } in node.events | During render-sync: `.on_mouse_up()`, `.on_key_down()`, etc. with a trampoline that calls back into Nim |
| 11 | `firstChild(node)` | Return node.children[0] if present | Shadow tree traversal only; no GPUI equivalent needed |
| 12 | `nextSibling(node)` | Find node in parent.children; return the next one | Shadow tree traversal only; no GPUI equivalent needed |
| 13 | `parentNode(node)` | Return node.parent | Shadow tree traversal only; no GPUI equivalent needed |

### Style Property Mapping (CSS to GPUI)

| CSS Property | GPUI Styled Method |
|-------------|-------------------|
| `width` | `.w(px(...))` or `.w_full()` |
| `height` | `.h(px(...))` or `.h_full()` |
| `background-color` | `.bg(rgb(...))` |
| `color` | `.text_color(rgb(...))` |
| `font-size` | `.text_sm()` / `.text_xl()` or custom |
| `padding` | `.p()` / `.px()` / `.py()` |
| `margin` | `.m()` / `.mx()` / `.my()` |
| `flex-direction: row` | `.flex_row()` |
| `flex-direction: column` | `.flex_col()` |
| `align-items: center` | `.items_center()` |
| `justify-content: center` | `.justify_center()` |
| `gap` | `.gap()` |
| `border-radius` | `.rounded()` / `.rounded_lg()` |
| `border` | `.border_1()` + `.border_color()` |
| `box-shadow` | `.shadow()` / `.shadow_lg()` |
| `opacity` | `.opacity()` |
| `cursor: pointer` | `.cursor_pointer()` |
| `overflow` | (limited support in GPUI) |
| `display: flex` | `.flex()` (default for div) |
| `display: none` | `.hidden()` |
| `position: absolute` | `.absolute()` |

---

## 6. Challenges and Limitations

### Declarative vs. Imperative Mismatch

**The fundamental challenge.** IsoNim's RendererBackend is imperative (DOM-like:
create nodes, append children, set attributes). GPUI is declarative (render functions
return element trees each frame). The shadow tree bridges this gap, but introduces
a render-sync step where the shadow tree must be translated into GPUI elements.

### No DOM-like API

GPUI has no `appendChild`, `removeChild`, or `insertBefore`. The element tree is
rebuilt from scratch each frame by calling `Render::render()`. Our shadow tree IS
the persistent state; the render-sync translates it to GPUI builders.

### Event Handler Threading

GPUI event handlers receive `&mut self` (the view), `&mut Window`, and
`&mut Context<Self>`. Our shim needs to translate these into C FFI callbacks that
Nim can handle. The current trampoline approach (16 fixed slots) works for
prototyping but will need a more scalable solution (e.g., a callback ID registry
with a single trampoline that dispatches by ID).

### Style Value Parsing

GPUI expects typed values (pixels, colors as RGBA, enum variants). CSS-style
string values like "16px", "#ff0000", "center" need to be parsed in the Rust shim
into the appropriate GPUI types. This parsing layer is non-trivial but well-bounded.

### Text Rendering Model

GPUI's text is not an "element" in the DOM sense. Text is added as a child of a div
via `.child("text")` or via `StyledText` for rich text. The shadow tree's `#text`
nodes need special handling during render-sync: they become string children of their
parent div, not standalone elements.

### Window Lifecycle

GPUI's `Application::run()` takes over the main thread and runs an event loop.
The Nim side cannot drive the event loop -- instead, `gpui_launch` must start the
GPUI application and use a callback to let Nim build the initial UI. Subsequent
updates happen via `cx.notify()` triggered by shadow tree mutations.

### Limited HTML Feature Coverage

Many HTML/CSS features have no GPUI equivalent:
- No `<table>` layout (use flexbox grid instead)
- No `<input>` / `<textarea>` (GPUI has separate input components in gpui-component)
- No CSS animations/transitions (GPUI has its own animation system)
- No CSS selectors or cascading (all styles are inline via builder methods)
- Limited overflow/scroll support compared to browsers

### Platform Dependencies

GPUI requires GPU access (Metal on macOS, Vulkan/OpenGL on Linux). The `gpui-backend`
feature flag controls whether actual rendering is compiled in. Without it, the shim
provides only the shadow tree for testing.

---

## 7. Design Decisions for the Shim

### Shadow Tree as Truth

The shadow tree in the Rust shim is the single source of truth for the UI state.
IsoNim's VDOM algorithm manipulates it imperatively. A separate render-sync step
(triggered by `cx.notify()`) walks the shadow tree and produces the GPUI element
tree for the current frame.

### Tag to Element Mapping

The Nim-side `renderer.nim` maps HTML tags to GPUI element names:
- Container tags (div, section, article, nav, etc.) -> `div()`
- Text tags (span, p, h1-h6, label, etc.) -> `div()` with text child
- Media tags (img) -> `img()`
- SVG tags (svg) -> `svg()`

### Style Translation Strategy

CSS property/value strings are parsed in the Rust shim during render-sync:
1. Property name is mapped (e.g., "background-color" -> bg)
2. Value is parsed (e.g., "#ff0000" -> rgb(0xff0000), "16px" -> px(16.0))
3. The appropriate Styled method is called on the GPUI element builder

### Event Flow

```
GPUI mouse/key event
  -> GPUI calls our root view's render() which includes event handlers
  -> Event handler looks up the shadow tree node for the target element
  -> Finds matching EventListener in the shadow tree
  -> Calls the C callback (trampoline)
  -> Nim handler runs, potentially mutating shadow tree state
  -> cx.notify() triggers re-render
  -> render-sync rebuilds GPUI element tree from updated shadow tree
```

### Two-Phase Implementation

- **Phase 1 (current, M0-M1):** Shadow tree only, no actual GPUI rendering.
  Tests verify the shadow tree operations work correctly.
- **Phase 2 (M2+):** Enable `gpui-backend` feature, implement render-sync,
  create actual GPUI windows.
