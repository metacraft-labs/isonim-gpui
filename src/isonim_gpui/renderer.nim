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

  # Text elements -> text
  "span": "text",
  "p": "text",
  "h1": "text",
  "h2": "text",
  "h3": "text",
  "h4": "text",
  "h5": "text",
  "h6": "text",
  "label": "text",
  "strong": "text",
  "em": "text",
  "small": "text",
  "code": "text",
  "pre": "text",

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
# Event callback bridge
# ===========================================================================

var callbackRegistry*: array[16, proc()]
var nextCallbackSlot: int

proc trampoline0() {.cdecl.} =
  if callbackRegistry[0] != nil: callbackRegistry[0]()
proc trampoline1() {.cdecl.} =
  if callbackRegistry[1] != nil: callbackRegistry[1]()
proc trampoline2() {.cdecl.} =
  if callbackRegistry[2] != nil: callbackRegistry[2]()
proc trampoline3() {.cdecl.} =
  if callbackRegistry[3] != nil: callbackRegistry[3]()
proc trampoline4() {.cdecl.} =
  if callbackRegistry[4] != nil: callbackRegistry[4]()
proc trampoline5() {.cdecl.} =
  if callbackRegistry[5] != nil: callbackRegistry[5]()
proc trampoline6() {.cdecl.} =
  if callbackRegistry[6] != nil: callbackRegistry[6]()
proc trampoline7() {.cdecl.} =
  if callbackRegistry[7] != nil: callbackRegistry[7]()
proc trampoline8() {.cdecl.} =
  if callbackRegistry[8] != nil: callbackRegistry[8]()
proc trampoline9() {.cdecl.} =
  if callbackRegistry[9] != nil: callbackRegistry[9]()
proc trampoline10() {.cdecl.} =
  if callbackRegistry[10] != nil: callbackRegistry[10]()
proc trampoline11() {.cdecl.} =
  if callbackRegistry[11] != nil: callbackRegistry[11]()
proc trampoline12() {.cdecl.} =
  if callbackRegistry[12] != nil: callbackRegistry[12]()
proc trampoline13() {.cdecl.} =
  if callbackRegistry[13] != nil: callbackRegistry[13]()
proc trampoline14() {.cdecl.} =
  if callbackRegistry[14] != nil: callbackRegistry[14]()
proc trampoline15() {.cdecl.} =
  if callbackRegistry[15] != nil: callbackRegistry[15]()

var trampolines: array[16, EventCallback] = [
  trampoline0, trampoline1, trampoline2, trampoline3,
  trampoline4, trampoline5, trampoline6, trampoline7,
  trampoline8, trampoline9, trampoline10, trampoline11,
  trampoline12, trampoline13, trampoline14, trampoline15,
]

const trampolineCount* = 16

proc registerCallback*(handler: proc()): EventCallback =
  assert nextCallbackSlot < trampolineCount,
    "GpuiRenderer: event callback trampoline pool exhausted (" &
    $trampolineCount & " slots)"
  let slot = nextCallbackSlot
  inc nextCallbackSlot
  callbackRegistry[slot] = handler
  trampolines[slot]

proc resetCallbacks*() =
  for i in 0 ..< trampolineCount:
    callbackRegistry[i] = nil
  nextCallbackSlot = 0

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
  let trampoline = registerCallback(handler)
  gpui_add_event_listener(node, event.cstring, trampoline)

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
