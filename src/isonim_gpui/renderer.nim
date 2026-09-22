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
  ## **THE IDENTITY, AND PLAT-38 MADE IT ONE.**
  ##
  ## This used to carry `of "disabled": "enabled"`, and `mapAttributeValue`
  ## below used to answer the LITERAL `"false"` for that name whatever the
  ## caller passed. Between them, `setAttribute(el, "disabled", "false")`
  ## stored `enabled="false"` — recording the element as DISABLED when the
  ## caller had said it was not — and `getAttribute(el, "disabled")` answered
  ## `""`, because `getAttribute` never mapped and the stored key was the
  ## other word. That is gap `PLAT21-VG2`, filed by `codetracer`'s PLAT-21
  ## against twelve vocabulary entries, and closing it is one of PLAT-38's
  ## three deliverables.
  ##
  ## **The function is kept rather than deleted, and that is a judgement
  ## call worth seeing.** Every arm was already the identity except the one
  ## that was wrong, so the honest replacement is `name`. It stays because it
  ## is the seam where a future renderer-specific rename would go, and
  ## because deleting it would make the diff a rename of two call sites
  ## instead of a repair anybody can read. It is now provably total: a
  ## round-trip assertion over every name a caller writes is in
  ## `tests/test_renderer.nim`, and the mutation arm that reintroduces the
  ## rewrite must redden it.
  name

proc mapAttributeValue(name, value: string): string =
  ## The identity. See `mapAttributeName` — the `disabled` constant-fold
  ## that lived here is `PLAT21-VG2`'s second half.
  ##
  ## A value mapping that inverted `disabled` into an `enabled` polarity —
  ## the remedy `gpui_gaps.nim` originally proposed — was considered and
  ## REJECTED. It would still mean `getAttribute(el, "disabled")` reading a
  ## key nobody wrote, so the round trip the deliverable asks for
  ## (*"an attribute round-trips under the name it was written with"*) would
  ## still fail; and the DOM, which is the medium the vocabulary's `data-*`
  ## facts were shaped against, keeps what it is given. Storing what the
  ## caller wrote is the smaller contract and the one a gate can state.
  discard name
  value

# ===========================================================================
# Event callback bridge — dynamic callback ID registry
# ===========================================================================
#
# Instead of a fixed pool of cdecl trampolines, we maintain a Table[int32, proc()]
# keyed by monotonically increasing callback IDs. A single global dispatcher
# (registered once with the Rust shim) looks up and invokes the closure.

type
  GpuiModifier* = enum
    ## The five modifiers GPUI itself distinguishes (`gpui::Modifiers`), in
    ## that struct's own declaration order so the mapping is a transcription
    ## rather than a decision.
    gmControl
    gmAlt
    gmShift
    gmPlatform
    gmFunction

  GpuiModifiers* = set[GpuiModifier]

  GpuiEventKind* = enum
    gekOther
    gekKeyDown
    gekKeyUp

  GpuiEvent* = object
    ## **What a handler is handed. PLAT-38's whole point.**
    ##
    ## The key is a BASE NAME plus a modifier SET — never a rendered
    ## `"Shift+F10"`. `codetracer-specs/Testing/Verification-Harness-Traps.md`
    ## §25: a helper that silently drops a modifier it cannot spell hands you
    ## a test about a different key, and this workspace has already paid for
    ## that once. A modifier cannot fall out of a set without the set
    ## changing, and the set is what the gate asserts.
    kind*: GpuiEventKind
    key*: string
    modifiers*: GpuiModifiers
    repeat*: bool

  GpuiEventHandler* = proc(ev: GpuiEvent)

func toModifiers*(bits: uint32): GpuiModifiers =
  ## Bitmask -> set. **One direction, one function**: `fromModifiers` is its
  ## inverse and `tests/test_renderer.nim` asserts the round trip over the
  ## whole power set, because two independently written converters are two
  ## chances to drop the same bit (§30).
  if (bits and GpuiModControl) != 0: result.incl gmControl
  if (bits and GpuiModAlt) != 0: result.incl gmAlt
  if (bits and GpuiModShift) != 0: result.incl gmShift
  if (bits and GpuiModPlatform) != 0: result.incl gmPlatform
  if (bits and GpuiModFunction) != 0: result.incl gmFunction

func fromModifiers*(mods: GpuiModifiers): uint32 =
  for m in mods:
    result = result or (case m
      of gmControl: GpuiModControl
      of gmAlt: GpuiModAlt
      of gmShift: GpuiModShift
      of gmPlatform: GpuiModPlatform
      of gmFunction: GpuiModFunction)

func toEventKind*(raw: uint32): GpuiEventKind =
  case raw
  of GpuiEventKeyDown: gekKeyDown
  of GpuiEventKeyUp: gekKeyUp
  else: gekOther

func fromEventKind*(k: GpuiEventKind): uint32 =
  case k
  of gekOther: GpuiEventOther
  of gekKeyDown: GpuiEventKeyDown
  of gekKeyUp: GpuiEventKeyUp

proc readPayload*(p: ptr GpuiEventPayload): GpuiEvent =
  ## Decode what the shim handed us. A nil pointer is the honest encoding of
  ## "this event carried nothing", and decodes to a `gekOther` with an empty
  ## key rather than raising — a click is not a malformed key.
  if p == nil: return GpuiEvent(kind: gekOther)
  GpuiEvent(
    kind: toEventKind(p.kind),
    key: (if p.key == nil: "" else: $p.key),
    modifiers: toModifiers(p.modifiers),
    repeat: p.repeat != 0)

var callbackTable*: Table[int32, GpuiEventHandler]
var nextCallbackId*: int32 = 1
var dispatcherRegistered: bool = false

proc globalDispatcher(callbackId: int32; payload: ptr GpuiEventPayload) {.cdecl.} =
  if callbackId in callbackTable:
    callbackTable[callbackId](readPayload(payload))

proc ensureDispatcherRegistered() =
  if not dispatcherRegistered:
    gpui_set_event_dispatcher(globalDispatcher)
    dispatcherRegistered = true

proc registerCallback*(handler: GpuiEventHandler): int32 =
  ## Register a Nim closure and return its callback ID.
  ensureDispatcherRegistered()
  let id = nextCallbackId
  inc nextCallbackId
  callbackTable[id] = handler
  id

proc registerCallback*(handler: proc()): int32 =
  ## The payload-blind form, for listeners that only care THAT the event
  ## happened — a click handler, most of this repo's own tests, and every
  ## call site that predates PLAT-38. It is an overload rather than the only
  ## form so that a handler which needs the key can ask for it; before
  ## PLAT-38 there was no such form at all, which is `PLAT21-VG1`.
  registerCallback(proc(ev: GpuiEvent) =
    discard ev
    handler())

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

proc addEventListener*(r: GpuiRenderer; node: GpuiElement; event: string;
                       handler: GpuiEventHandler) =
  ## **The payload-carrying overload — `PLAT21-VG1`'s remedy, taken.**
  ##
  ## The handler receives the event. Before PLAT-38 the only signature this
  ## renderer offered was `proc()`, so the sole channel that could say WHICH
  ## key was pressed was the event NAME, and `codetracer`'s GPUI binding
  ## registered one listener per key of an entry's contract under
  ## `vockey:<name>`. Nothing here forbids that spelling; what changed is
  ## that a binding no longer has to invent one.
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

proc nodeId*(node: GpuiElement): uint64 =
  ## Stable identity of the shadow-tree node behind an opaque handle.
  ## See `bindings.gpui_node_id` for why handle pointers cannot be
  ## compared directly.
  if node == nil: 0'u64 else: gpui_node_id(node)

proc sameNode*(a, b: GpuiElement): bool =
  ## "Are these two handles the same node?" — the comparison every
  ## identity assertion in this repo must use. `a == b` compares
  ## POINTERS and is false for two handles to one node.
  let ia = nodeId(a)
  ia != 0'u64 and ia == nodeId(b)

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
# PLAT-38 — delivering a key, and element focus
# ===========================================================================

template withPayload(ev: GpuiEvent; body: untyped): untyped =
  ## Build a `GpuiEventPayload` whose `key` points into a cstring that is
  ## alive for the whole of `body`, and expose it as `payloadPtr`.
  ##
  ## A template rather than a proc returning the struct: the payload holds a
  ## RAW POINTER into `keyBuf`, and a proc's local would be collectable the
  ## moment it returned. That is the kind of defect that works in every test
  ## and fails under load, so the lifetime is made structural.
  block:
    let keyBuf {.inject.} = ev.key.cstring
    var payload {.inject.} = GpuiEventPayload(
      kind: fromEventKind(ev.kind),
      modifiers: fromModifiers(ev.modifiers),
      key: (if ev.key.len == 0: nil else: keyBuf),
      repeat: (if ev.repeat: 1'u8 else: 0'u8))
    let payloadPtr {.inject.} = addr payload
    body

proc fireEvent*(node: GpuiElement; event: string; ev: GpuiEvent): int =
  ## Dispatch `event` at `node` carrying `ev`. **Answers how many listeners
  ## it reached**, which is the difference between a key that reached
  ## nothing and a key that reached something — both of which satisfy "the
  ## event fired".
  withPayload(ev):
    result = int(gpui_dispatch_event_with(node, event.cstring, payloadPtr))

proc sendKeyToFocus*(event: string; ev: GpuiEvent): int =
  ## Route a key the way a window does: to whatever element holds focus.
  ## Answers 0 when nothing holds focus, and 0 when no WINDOW holds focus.
  withPayload(ev):
    result = int(gpui_dispatch_key_to_focus(event.cstring, payloadPtr))

proc readStoreString(node: GpuiElement;
                     get: proc(n: GpuiElement; buf: pointer;
                               bufLen: uint64): uint64 {.cdecl.}): string =
  ## The two-call sizing protocol, once. `textContent`, `getAttribute` and
  ## `getTag` above each spell it out again; the three are left alone (they
  ## are not this milestone's to rewrite) and the new readers share this.
  let needed = get(node, nil, 0)
  if needed == 0: return ""
  var buf = newString(int(needed) + 1)
  discard get(node, addr buf[0], uint64(buf.len))
  buf.setLen(int(needed))
  buf

proc lastEventKey*(node: GpuiElement): string =
  ## **The base key name the ELEMENT STORE recorded**, read back across the
  ## FFI boundary. Written by the shim before any callback ran, so a Nim
  ## handler cannot forge it and a binding that applied the key on its own
  ## side leaves it empty.
  readStoreString(node, gpui_last_event_key)

proc lastEventName*(node: GpuiElement): string =
  readStoreString(node, gpui_last_event_name)

proc lastEventModifiers*(node: GpuiElement): GpuiModifiers =
  toModifiers(gpui_last_event_modifiers(node))

proc lastEventKind*(node: GpuiElement): GpuiEventKind =
  toEventKind(gpui_last_event_kind(node))

proc lastEventRepeat*(node: GpuiElement): bool =
  gpui_last_event_repeat(node) != 0

proc lastEventSeq*(node: GpuiElement): int =
  ## 0 means nothing ever arrived at this element.
  int(gpui_last_event_seq(node))

proc lastEvent*(node: GpuiElement): GpuiEvent =
  ## The whole record, as one value.
  GpuiEvent(kind: lastEventKind(node), key: lastEventKey(node),
            modifiers: lastEventModifiers(node), repeat: lastEventRepeat(node))

proc deliveryCount*(node: GpuiElement): int =
  int(gpui_event_delivery_count(node))

proc setFocusable*(node: GpuiElement; focusable = true) =
  gpui_set_focusable(node, (if focusable: 1'u8 else: 0'u8))

proc isFocusable*(node: GpuiElement): bool =
  gpui_is_focusable(node) != 0

proc focusElement*(node: GpuiElement): bool =
  ## `true` if the element took focus, `false` if the request was REFUSED.
  ## A trap that could be escaped by asking would not be one.
  gpui_focus_element(node) != 0

proc blurElement*(node: GpuiElement) =
  gpui_blur_element(node)

proc isFocused*(node: GpuiElement): bool =
  gpui_is_focused(node) != 0

proc focusedElement*(): GpuiElement =
  gpui_focused_element()

proc focusedCount*(): int =
  ## **The focus partition law's instrument.** Counted over the whole
  ## element store rather than over the element the caller just focused;
  ## see `rust/gpui-nim-shim/src/tree.rs`'s `Node::focused` for why the
  ## implementation is shaped so this number CAN be 2.
  int(gpui_focused_count())

proc setFocusTrap*(node: GpuiElement; trap = true): bool =
  gpui_set_focus_trap(node, (if trap: 1'u8 else: 0'u8)) != 0

proc focusTrapElement*(): GpuiElement =
  gpui_focus_trap_element()

proc focusableCount*(): int =
  int(gpui_focusable_count())

proc focusableAt*(index: int): GpuiElement =
  gpui_focusable_at(uint64(index))

proc focusOrder*(): seq[GpuiElement] =
  ## The declared focus order, as the Rust side reports it.
  for i in 0 ..< focusableCount():
    result.add focusableAt(i)

proc focusNext*(): bool =
  gpui_focus_next() != 0

proc focusPrev*(): bool =
  gpui_focus_prev() != 0

proc getTag*(node: GpuiElement): string =
  ## Read the element's tag name (e.g. "div", "button", "p"). Returns
  ## "" for nil nodes or text nodes (which have no tag). Used by the
  ## RS-M2 GPUI streaming adapter (in `isonim-render-serve`) to derive
  ## a per-element fill colour when rasterizing the headless tree to
  ## RGBA pixels.
  if node == nil: return ""
  let needed = gpui_get_tag(node, nil, 0)
  if needed == 0: return ""
  var buf = newString(int(needed) + 1)
  discard gpui_get_tag(node, addr buf[0], uint64(buf.len))
  buf.setLen(int(needed))
  buf

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
