## GpuiRenderer — implements IsoNim's RendererBackend backed by GPUI,
## Zed's GPU-accelerated UI framework, via the Rust FFI shim.
##
## This module wraps the raw C bindings in bindings.nim into the
## high-level proc signatures that checkRendererBackend expects.
##
## Design:
## - HTML-like tags are mapped to GPUI element names (div, text, img)
## - CSS-like style properties are mapped to GPUI's Tailwind-inspired styling
## - HTML attributes are mapped to GPUI-appropriate attributes
## - Event listeners use a callback registry + cdecl trampoline bridge
##
## GPUI uses a Tailwind-inspired styling system with methods like
## .bg(), .text_color(), .w(), .h(), .flex(), etc. The Rust shim
## translates the string-based style properties into these calls.

import std/tables
import isonim_gpui/bindings

# Re-export GpuiElement so users only need to import renderer.
export bindings.GpuiElement

type
  GpuiRenderer* = object
    ## Renderer backend that delegates to GPUI via FFI.

# ===========================================================================
# Tag mapping: HTML tags -> GPUI element model
# ===========================================================================
#
# GPUI's element model is based on:
# - div() — flexbox container (the primary layout primitive)
# - text() / StyledText — text rendering
# - img() — image rendering
# - svg() — SVG rendering
#
# Most HTML container-like tags map to div(), text tags map to "text".

const tagMap = {
  # Generic containers -> div
  "div": "div",
  "section": "div",
  "article": "div",
  "main": "div",
  "aside": "div",
  "nav": "div",
  "header": "div",
  "footer": "div",
  "form": "div",
  "details": "div",
  "summary": "div",
  "fieldset": "div",

  # Text elements -> pass through original HTML tag so Rust classifies as TextContainer
  "span": "span",
  "p": "p",
  "h1": "h1",
  "h2": "h2",
  "h3": "h3",
  "h4": "h4",
  "h5": "h5",
  "h6": "h6",
  "label": "label",
  "strong": "strong",
  "em": "em",
  "small": "small",
  "code": "code",
  "pre": "pre",

  # Interactive -> div (with event handling)
  "button": "div",
  "input": "div",
  "textarea": "div",
  "select": "div",

  # Lists -> div
  "ul": "div",
  "ol": "div",
  "li": "div",

  # Media
  "img": "img",
  "svg": "svg",
}.toTable

proc mapTag(tag: string): string =
  ## Map an HTML-like tag to the corresponding GPUI element name.
  ## Unknown tags pass through as-is (allows using GPUI-native names directly).
  if tag in tagMap:
    tagMap[tag]
  else:
    tag

# ===========================================================================
# Style mapping: CSS properties -> GPUI style properties
# ===========================================================================
#
# GPUI uses a Tailwind-inspired styling API:
# - Dimensions: .w(), .h(), .min_w(), .max_w(), etc.
# - Spacing: .p(), .px(), .py(), .m(), .mx(), .my()
# - Colors: .bg(), .text_color(), .border_color()
# - Layout: .flex(), .flex_row(), .flex_col(), .items_center(), .justify_center()
# - Typography: .text_size(), .font_weight(), .font_family()
#
# The Rust shim receives string property names and values, then maps them
# to the appropriate GPUI method calls.

const stylePropertyMap = {
  # Dimensions
  "width": "width",
  "height": "height",
  "min-width": "min_width",
  "max-width": "max_width",
  "min-height": "min_height",
  "max-height": "max_height",

  # Spacing
  "padding": "padding",
  "padding-top": "padding_top",
  "padding-bottom": "padding_bottom",
  "padding-left": "padding_left",
  "padding-right": "padding_right",
  "margin": "margin",
  "margin-top": "margin_top",
  "margin-bottom": "margin_bottom",
  "margin-left": "margin_left",
  "margin-right": "margin_right",

  # Colors
  "background-color": "bg",
  "background": "bg",
  "color": "text_color",

  # Typography
  "font-size": "text_size",
  "font-family": "font_family",
  "font-weight": "font_weight",
  "font-style": "font_style",
  "text-align": "text_align",
  "line-height": "line_height",

  # Layout
  "flex-direction": "flex_direction",
  "align-items": "align_items",
  "justify-content": "justify_content",
  "gap": "gap",
  "flex-grow": "flex_grow",
  "flex-shrink": "flex_shrink",
  "flex-basis": "flex_basis",

  # Border
  "border": "border",
  "border-radius": "corner_radius",
  "border-color": "border_color",
  "border-width": "border_width",

  # Misc
  "overflow": "overflow",
  "opacity": "opacity",
  "cursor": "cursor",

  # Shadow
  "box-shadow": "shadow",
}.toTable

proc mapStyleProperty(prop: string): string =
  if prop in stylePropertyMap:
    stylePropertyMap[prop]
  else:
    prop

proc mapStyleValue(prop, value: string): string =
  ## Map a CSS-like style value to GPUI's expected format.
  case prop
  of "flex-direction":
    case value
    of "row", "row-reverse": "row"
    of "column", "column-reverse": "col"
    else: value
  of "align-items":
    case value
    of "flex-start", "start": "start"
    of "flex-end", "end": "end"
    of "center": "center"
    of "stretch": "stretch"
    else: value
  of "justify-content":
    case value
    of "flex-start", "start": "start"
    of "flex-end", "end": "end"
    of "center": "center"
    of "space-between": "space_between"
    of "space-around": "space_around"
    of "space-evenly": "space_evenly"
    else: value
  else:
    value

# ===========================================================================
# Attribute mapping
# ===========================================================================

proc mapAttributeName(name: string): string =
  case name
  of "class": "class"
  of "id": "id"
  of "placeholder": "placeholder"
  of "value": "value"
  of "disabled": "enabled"
  of "href": "href"
  of "src": "src"
  of "alt": "alt"
  of "title": "title"
  of "type": "type"
  of "name": "name"
  else: name

proc mapAttributeValue(name, value: string): string =
  case name
  of "disabled": "false"
  else: value

# ===========================================================================
# Event callback bridge — dynamic callback ID registry
# ===========================================================================
#
# Instead of a fixed pool of cdecl trampolines, we maintain a Table[int32, proc()]
# keyed by monotonically increasing callback IDs. A single global dispatcher
# (registered once with the Rust shim) looks up and invokes the closure.

var callbackTable*: Table[int32, proc()]
var nextCallbackId*: int32 = 1
var dispatcherRegistered: bool = false

proc globalDispatcher(callbackId: int32) {.cdecl.} =
  if callbackId in callbackTable:
    callbackTable[callbackId]()

proc ensureDispatcherRegistered() =
  if not dispatcherRegistered:
    gpui_set_event_dispatcher(globalDispatcher)
    dispatcherRegistered = true

proc registerCallback*(handler: proc()): int32 =
  ## Register a Nim closure and return its callback ID.
  ensureDispatcherRegistered()
  let id = nextCallbackId
  inc nextCallbackId
  callbackTable[id] = handler
  id

proc removeCallback*(id: int32) =
  ## Remove a callback from the registry by its ID.
  callbackTable.del(id)

proc resetCallbacks*() =
  callbackTable.clear()
  nextCallbackId = 1
  # Re-register the dispatcher to ensure it's always set after reset
  gpui_set_event_dispatcher(globalDispatcher)
  dispatcherRegistered = true

# ===========================================================================
# RendererBackend implementation (13 procs)
# ===========================================================================

proc createElement*(r: GpuiRenderer; tag: string): GpuiElement =
  let gpuiTag = mapTag(tag)
  gpui_create_element(gpuiTag.cstring)

proc createTextNode*(r: GpuiRenderer; text: string): GpuiElement =
  gpui_create_text_node(text.cstring)

proc appendChild*(r: GpuiRenderer; parent, child: GpuiElement) =
  gpui_append_child(parent, child)

proc insertBefore*(r: GpuiRenderer; parent, child, reference: GpuiElement) =
  gpui_insert_before(parent, child, reference)

proc removeChild*(r: GpuiRenderer; parent, child: GpuiElement) =
  gpui_remove_child(parent, child)

proc setAttribute*(r: GpuiRenderer; node: GpuiElement; name, value: string) =
  let gpuiName = mapAttributeName(name)
  let gpuiValue = mapAttributeValue(name, value)
  gpui_set_attribute(node, gpuiName.cstring, gpuiValue.cstring)

proc removeAttribute*(r: GpuiRenderer; node: GpuiElement; name: string) =
  let gpuiName = mapAttributeName(name)
  gpui_remove_attribute(node, gpuiName.cstring)

proc setTextContent*(r: GpuiRenderer; node: GpuiElement; text: string) =
  gpui_set_text_content(node, text.cstring)

proc setStyle*(r: GpuiRenderer; node: GpuiElement; prop, value: string) =
  let gpuiProp = mapStyleProperty(prop)
  let gpuiValue = mapStyleValue(prop, value)
  gpui_set_style(node, gpuiProp.cstring, gpuiValue.cstring)

proc addEventListener*(r: GpuiRenderer; node: GpuiElement; event: string; handler: proc()) =
  let callbackId = registerCallback(handler)
  gpui_add_event_listener_id(node, event.cstring, callbackId)

proc firstChild*(r: GpuiRenderer; node: GpuiElement): GpuiElement =
  gpui_first_child(node)

proc nextSibling*(r: GpuiRenderer; node: GpuiElement): GpuiElement =
  gpui_next_sibling(node)

proc parentNode*(r: GpuiRenderer; node: GpuiElement): GpuiElement =
  gpui_parent_node(node)

# ===========================================================================
# Tree inspection helpers (for testing / cross-renderer comparison)
# ===========================================================================

proc childCount*(node: GpuiElement): int =
  int(gpui_child_count(node))

proc textContent*(node: GpuiElement): string =
  let needed = gpui_get_text_content(node, nil, 0)
  if needed == 0:
    return ""
  var buf = newString(int(needed) + 1)
  discard gpui_get_text_content(node, addr buf[0], uint64(buf.len))
  buf.setLen(int(needed))
  buf

proc getAttribute*(node: GpuiElement; name: string): string =
  let needed = gpui_get_attribute(node, name.cstring, nil, 0)
  if needed == 0:
    return ""
  var buf = newString(int(needed) + 1)
  discard gpui_get_attribute(node, name.cstring, addr buf[0], uint64(buf.len))
  buf.setLen(int(needed))
  buf

proc nthChild*(node: GpuiElement; index: int): GpuiElement =
  gpui_nth_child(node, uint64(index))

proc fireEvent*(node: GpuiElement; event: string) =
  gpui_dispatch_event(node, event.cstring)

# ===========================================================================
# Render plan inspection (G3-G — integration testing)
# ===========================================================================

proc renderPlanJson*(r: GpuiRenderer; root: GpuiElement): string =
  ## Build a render plan from the shadow tree rooted at `root` and return
  ## it as a JSON string. Returns "" if the node is nil or missing.
  let raw = gpui_render_plan_json(root)
  if raw == nil:
    return ""
  let cstr = cast[cstring](raw)
  result = $cstr
  gpui_free_string(raw)

proc renderPlanElementCount*(r: GpuiRenderer; root: GpuiElement): int =
  ## Return the total number of elements in the render plan rooted at `root`.
  int(gpui_render_plan_element_count(root))

proc verifyRenderPlan*(r: GpuiRenderer; root: GpuiElement): bool =
  ## Return true if a valid render plan can be built from the shadow tree
  ## rooted at `root`.
  gpui_verify_render_plan(root) == 1

# ===========================================================================
# Compile-time concept check
# ===========================================================================

static:
  var r: GpuiRenderer
  var e: GpuiElement
  assert compiles(r.createElement(""))
  assert compiles(r.createTextNode(""))
  assert compiles(r.appendChild(e, e))
  assert compiles(r.insertBefore(e, e, e))
  assert compiles(r.removeChild(e, e))
  assert compiles(r.setAttribute(e, "", ""))
  assert compiles(r.removeAttribute(e, ""))
  assert compiles(r.setTextContent(e, ""))
  assert compiles(r.setStyle(e, "", ""))
  assert compiles(r.addEventListener(e, "", proc() = discard))
  assert compiles(r.firstChild(e))
  assert compiles(r.nextSibling(e))
  assert compiles(r.parentNode(e))
