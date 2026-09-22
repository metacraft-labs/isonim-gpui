## PLAT-38 — the widened event ABI and element focus, against the REAL shim.
##
## `test_bindings.nim` checks that every binding has the right SIGNATURE and
## links nothing. This file links `libgpui_nim_shim.so` and drives it, so
## every assertion below is about what the Rust-side element store actually
## holds after a call crossed the FFI boundary.
##
## ## What each group is for
##
## 1. **The ABI layout.** A `#[repr(C)]` struct passed by pointer does not
##    fail to LINK when the two sides disagree about its shape; it delivers a
##    different key. The Rust compiler's own `size_of` / `offset_of` answers
##    come back through `gpui_event_payload_layout` and are compared against
##    Nim's, so neither side is graded against a literal the other also
##    carries.
##
## 2. **Key delivery** (`PLAT21-VG1`). The payload reaches the handler AND
##    the element store, by two different routes, and the two agree.
##
## 3. **The attribute round trip** (`PLAT21-VG2`), both polarities, under the
##    name the caller wrote, plus the ABSENCE of the rewrite asserted with a
##    planted positive control — an absence check with no control is
##    `codetracer-specs/Testing/Verification-Harness-Traps.md` §4.
##
## 4. **Element focus** (`PLAT21-VG3`): the partition, the declared order in
##    both directions, and the trap that makes `Modal` expressible.
##
## No mocks. `GpuiRenderer` is the real renderer and the tree is the real
## Rust shim's.

import std/[strutils, unittest]

import isonim_gpui/bindings
import isonim_gpui/renderer

const ExpectedAssertions = 184
  ## Asserted against the runtime tally below and printed as `CHECKS:`.

var checkCount = 0

template ck(cond: untyped) =
  ## Every `check` in this file goes through here so the tally cannot drift
  ## from the assertions. It is a TEMPLATE and not a proc: a `unittest.check`
  ## inside a plain `proc` writes the module-level `testStatusIMPL`, so the
  ## test reports `[OK]` with the failed comparison printed above it (§29).
  inc checkCount
  check cond

proc freshTree() =
  gpui_reset_tree()
  gpui_reset_windows()
  resetCallbacks()

let r = GpuiRenderer()

# ---------------------------------------------------------------------------

suite "PLAT-38 — the payload ABI":

  test "the two sides agree about GpuiEventPayload's layout":
    # The Rust side is asked; Nim answers from its own compiler. A literal
    # shared by both would be satisfied by both being wrong.
    ck int(gpui_event_payload_layout(0)) == sizeof(GpuiEventPayload)
    ck int(gpui_event_payload_layout(1)) == alignof(GpuiEventPayload)
    ck int(gpui_event_payload_layout(2)) == offsetOf(GpuiEventPayload, kind)
    ck int(gpui_event_payload_layout(3)) == offsetOf(GpuiEventPayload, modifiers)
    ck int(gpui_event_payload_layout(4)) == offsetOf(GpuiEventPayload, key)
    ck int(gpui_event_payload_layout(5)) == offsetOf(GpuiEventPayload, repeat)
    # And the reader is two-sided about its own input: a layout query the
    # shim does not understand must be unmistakable, or "0" would read as a
    # legitimate offset.
    ck gpui_event_payload_layout(99) == high(uint64)

  test "the modifier set round-trips over its whole power set":
    # `toModifiers` and `fromModifiers` are the ONE pair; a second spelling
    # anywhere would be two chances to drop the same bit (§30). Over the
    # power set rather than over a handful, because a converter that drops a
    # bit drops it in exactly one combination class.
    var seen = 0
    for bits in 0'u32 .. 31'u32:
      ck fromModifiers(toModifiers(bits)) == bits
      inc seen
    # §4: the sweep asserted 32 things; say so, so a loop that stopped
    # iterating cannot satisfy every claim written over it.
    ck seen == 32

  test "every event kind round-trips and an unknown kind is not a key":
    for k in GpuiEventKind:
      ck toEventKind(fromEventKind(k)) == k
    ck toEventKind(4242'u32) == gekOther

  test "a nil payload decodes to an event that carries nothing":
    let ev = readPayload(nil)
    ck ev.kind == gekOther
    ck ev.key == ""
    ck ev.modifiers == {}
    ck not ev.repeat

# ---------------------------------------------------------------------------

suite "PLAT-38 — a key reaches a view with its payload (PLAT21-VG1)":

  setup:
    freshTree()

  test "the handler and the element store see the same key":
    let el = r.createElement("button")
    var seen: GpuiEvent
    var ran = 0
    r.addEventListener(el, "keydown", proc(ev: GpuiEvent) =
      seen = ev
      inc ran)
    let sent = GpuiEvent(kind: gekKeyDown, key: "F10",
                         modifiers: {gmShift, gmControl}, repeat: true)
    let reached = fireEvent(el, "keydown", sent)
    ck reached == 1
    ck ran == 1
    # The handler's copy came back through a function pointer…
    ck seen.key == "F10"
    ck seen.modifiers == {gmShift, gmControl}
    ck seen.kind == gekKeyDown
    ck seen.repeat
    # …and this one came out of the Rust store. A dispatcher that handed the
    # handler a fresh zeroed struct would pass a store-only check.
    ck el.lastEventKey() == "F10"
    ck el.lastEventModifiers() == {gmShift, gmControl}
    ck el.lastEventKind() == gekKeyDown
    ck el.lastEventRepeat()
    ck el.lastEventName() == "keydown"
    ck el.deliveryCount() == 1

  test "a key with no listener ARRIVES and reaches nothing":
    # Both states satisfy "the event fired"; the gate has to tell them apart.
    let el = r.createElement("div")
    let reached = fireEvent(el, "keydown",
      GpuiEvent(kind: gekKeyDown, key: "Up"))
    ck reached == 0
    ck el.deliveryCount() == 0
    ck el.lastEventSeq() > 0
    ck el.lastEventKey() == "Up"

  test "a node that received nothing answers a sequence of zero":
    let el = r.createElement("div")
    ck el.lastEventSeq() == 0
    ck el.lastEventKey() == ""
    ck el.deliveryCount() == 0

  test "the same key twice is two deliveries, not one":
    # An equality over key names alone cannot see this: both deliveries carry
    # the same key. The sequence number is what separates "nothing arrived"
    # from "the same thing arrived again".
    let el = r.createElement("button")
    r.addEventListener(el, "keydown", proc() = discard)
    discard fireEvent(el, "keydown", GpuiEvent(kind: gekKeyDown, key: "a"))
    let first = el.lastEventSeq()
    discard fireEvent(el, "keydown", GpuiEvent(kind: gekKeyDown, key: "a"))
    let second = el.lastEventSeq()
    ck first > 0
    ck second > first
    ck el.deliveryCount() == 2

  test "a non-ASCII key survives the boundary intact":
    # The key crosses as UTF-8 bytes. A boundary that truncated at the first
    # non-ASCII byte, or that round-tripped through a narrow char type, would
    # be invisible to a corpus of arrow keys.
    let el = r.createElement("input")
    var seen = ""
    r.addEventListener(el, "keydown", proc(ev: GpuiEvent) = seen = ev.key)
    for key in ["é", "ß", "日", "🙂"]:
      discard fireEvent(el, "keydown", GpuiEvent(kind: gekKeyDown, key: key))
      ck seen == key
      ck el.lastEventKey() == key

  test "an unmodified key is distinguishable from a modified one":
    # §25, stated as an assertion rather than as a caution: a transport that
    # dropped a modifier would make these two the same observation.
    let el = r.createElement("button")
    r.addEventListener(el, "keydown", proc() = discard)
    discard fireEvent(el, "keydown", GpuiEvent(kind: gekKeyDown, key: "Tab"))
    ck el.lastEventModifiers() == {}
    discard fireEvent(el, "keydown",
      GpuiEvent(kind: gekKeyDown, key: "Tab", modifiers: {gmShift}))
    ck el.lastEventModifiers() == {gmShift}

  test "a payload-blind listener still runs, and the store still records":
    # The `proc()` overload is what every call site older than PLAT-38 uses.
    # It must keep working AND must not cost the store its record.
    let el = r.createElement("button")
    var ran = 0
    r.addEventListener(el, "click", proc() = inc ran)
    fireEvent(el, "click")
    ck ran == 1
    ck el.deliveryCount() == 1
    ck el.lastEventName() == "click"
    ck el.lastEventKey() == ""
    ck el.lastEventKind() == gekOther

# ---------------------------------------------------------------------------

suite "PLAT-38 — an attribute round-trips under its own name (PLAT21-VG2)":

  setup:
    freshTree()

  test "disabled round-trips in both polarities":
    let el = r.createElement("button")
    for v in ["true", "false"]:
      r.setAttribute(el, "disabled", v)
      ck getAttribute(el, "disabled") == v
    # And the name the renderer used to substitute is not there at all.
    ck getAttribute(el, "enabled") == ""

  test "removing disabled removes the name that was written":
    let el = r.createElement("button")
    r.setAttribute(el, "disabled", "true")
    ck getAttribute(el, "disabled") == "true"
    r.removeAttribute(el, "disabled")
    ck getAttribute(el, "disabled") == ""

  test "every attribute a caller writes round-trips":
    # The names `mapAttributeName` used to enumerate, plus two it never did.
    # A repair that fixed `disabled` by special-casing it would pass the case
    # above and fail here on a name nobody thought about.
    let el = r.createElement("input")
    var round = 0
    for name in ["class", "id", "placeholder", "value", "disabled", "href",
                 "src", "alt", "title", "type", "name", "data-option-id",
                 "checked"]:
      let v = "v-" & name
      r.setAttribute(el, name, v)
      ck getAttribute(el, name) == v
      inc round
    ck round == 13

  test "the disabled rewrite is ABSENT from the renderer, with a control":
    # §4: an absence grep with no positive control is satisfied by a pattern
    # that cannot match. The control is a spelling that IS in the file.
    const src = staticRead("../src/isonim_gpui/renderer.nim")
    var code = ""
    for line in src.splitLines():
      let t = line.strip()
      if t.startsWith("#") or t.startsWith("##"):
        continue
      code.add line & "\n"
    # THE POSITIVE CONTROL, first: the scan is reading the file.
    ck code.contains("proc mapAttributeName")
    ck code.contains("proc setAttribute*")
    ck code.len > 2000
    # THE ABSENCE. The needle is the rewrite's own shape, not the word
    # `enabled` — which is a legitimate English word this file's prose uses.
    ck not code.contains("of \"disabled\": \"enabled\"")
    ck not code.contains("of \"disabled\": \"false\"")

# ---------------------------------------------------------------------------

suite "PLAT-38 — element focus (PLAT21-VG3)":

  setup:
    freshTree()

  test "focus is refused where it was not declared":
    let el = r.createElement("div")
    gpui_set_root_element(el)
    ck not focusElement(el)
    ck focusedCount() == 0

  test "at most one element holds focus, counted over the whole tree":
    # THE FOCUS PARTITION. Read from `gpui_focused_count`, which walks every
    # node in the store — not from the element this case just focused. The
    # published killer ("let two elements hold it") is performable against
    # this implementation, which is why the law is a check.
    let root = r.createElement("div")
    var buttons: seq[GpuiElement]
    for i in 0 .. 4:
      let b = r.createElement("button")
      setFocusable(b)
      r.appendChild(root, b)
      buttons.add b
    gpui_set_root_element(root)
    ck focusedCount() == 0
    for b in buttons:
      ck focusElement(b)
      ck focusedCount() == 1
      ck isFocused(b)
    blurElement(buttons[^1])
    ck focusedCount() == 0

  test "the declared order is document order and is readable from Rust":
    let root = r.createElement("div")
    let a = r.createElement("button")
    let mid = r.createElement("div")
    let b = r.createElement("input")
    let c = r.createElement("button")
    r.appendChild(root, a)
    r.appendChild(root, mid)
    r.appendChild(mid, b)
    r.appendChild(root, c)
    gpui_set_root_element(root)
    for el in [a, b, c]:
      setFocusable(el)
    ck focusableCount() == 3
    let order = focusOrder()
    ck order.len == 3
    ck sameNode(order[0], a)
    ck sameNode(order[1], b)   # nested, and still second — document order
    ck sameNode(order[2], c)

  test "focus moves forward through the declared order and wraps":
    let root = r.createElement("div")
    let a = r.createElement("button")
    let b = r.createElement("button")
    let c = r.createElement("button")
    for el in [a, b, c]:
      r.appendChild(root, el)
      setFocusable(el)
    gpui_set_root_element(root)
    ck focusElement(a)
    ck focusNext()
    ck isFocused(b)
    ck focusNext()
    ck isFocused(c)
    ck focusNext()
    ck isFocused(a)          # wrapped, rather than clamped (§36a)
    ck focusedCount() == 1

  test "focus moves backward through the declared order and wraps":
    let root = r.createElement("div")
    let a = r.createElement("button")
    let b = r.createElement("button")
    let c = r.createElement("button")
    for el in [a, b, c]:
      r.appendChild(root, el)
      setFocusable(el)
    gpui_set_root_element(root)
    ck focusElement(a)
    ck focusPrev()
    ck isFocused(c)          # wrapped the other way
    ck focusPrev()
    ck isFocused(b)
    ck focusedCount() == 1

  test "a focus trap confines the order and refuses the outside":
    # This is what makes `Modal` expressible: the entry's specified behaviour
    # is exclusive input until dismissed, and a trap is that statement in the
    # renderer rather than in prose.
    let root = r.createElement("div")
    let outside = r.createElement("button")
    let modal = r.createElement("dialog")
    let m1 = r.createElement("button")
    let m2 = r.createElement("button")
    r.appendChild(root, outside)
    r.appendChild(root, modal)
    r.appendChild(modal, m1)
    r.appendChild(modal, m2)
    gpui_set_root_element(root)
    for el in [outside, m1, m2]:
      setFocusable(el)
    ck focusableCount() == 3
    ck focusElement(outside)
    ck setFocusTrap(modal)
    # Focus was outside; setting the trap moved it inside.
    ck isFocused(m1)
    ck not isFocused(outside)
    ck focusableCount() == 2
    # The outside refuses, rather than quietly taking focus.
    ck not focusElement(outside)
    ck isFocused(m1)
    # And motion cycles inside the trap only.
    ck focusNext()
    ck isFocused(m2)
    ck focusNext()
    ck isFocused(m1)
    ck focusedCount() == 1
    # Dismissed: the trap releases and the outside is reachable again.
    ck setFocusTrap(modal, false)
    ck focusableCount() == 3
    ck focusElement(outside)
    ck isFocused(outside)

  test "a key routes to the focused element and to nothing else":
    let root = r.createElement("div")
    let a = r.createElement("button")
    let b = r.createElement("button")
    for el in [a, b]:
      r.appendChild(root, el)
      setFocusable(el)
      r.addEventListener(el, "keydown", proc() = discard)
    gpui_set_root_element(root)
    let win = gpui_create_window("t", 100, 100)
    discard gpui_show_window(win)
    gpui_notify_focus(win, 1)
    ck focusElement(a)
    let reached = sendKeyToFocus("keydown",
      GpuiEvent(kind: gekKeyDown, key: "Down"))
    ck reached == 1
    ck a.lastEventKey() == "Down"
    ck a.deliveryCount() == 1
    # THE ELEMENT THAT DID NOT HOLD FOCUS. "The key arrived" is true of a
    # binding that dispatches to everything.
    ck b.lastEventSeq() == 0
    ck b.deliveryCount() == 0

  test "THE NEGATIVE TWIN — the same key with the window unfocused":
    let root = r.createElement("div")
    let a = r.createElement("button")
    r.appendChild(root, a)
    setFocusable(a)
    r.addEventListener(a, "keydown", proc() = discard)
    gpui_set_root_element(root)
    let win = gpui_create_window("t", 100, 100)
    discard gpui_show_window(win)
    ck focusElement(a)
    # The window has never been told it holds focus.
    ck sendKeyToFocus("keydown", GpuiEvent(kind: gekKeyDown, key: "Down")) == 0
    ck a.lastEventSeq() == 0
    # The SAME call, with the only variable changed, does arrive — so the
    # zero above is about window focus rather than about anything else.
    gpui_notify_focus(win, 1)
    ck sendKeyToFocus("keydown", GpuiEvent(kind: gekKeyDown, key: "Down")) == 1
    ck a.lastEventKey() == "Down"
    # And it stops again when focus leaves.
    gpui_notify_focus(win, 0)
    let before = a.lastEventSeq()
    ck sendKeyToFocus("keydown", GpuiEvent(kind: gekKeyDown, key: "Up")) == 0
    ck a.lastEventSeq() == before

  test "a key routed with nothing focused reaches nothing":
    let root = r.createElement("div")
    let a = r.createElement("button")
    r.appendChild(root, a)
    setFocusable(a)
    gpui_set_root_element(root)
    let win = gpui_create_window("t", 100, 100)
    discard gpui_show_window(win)
    gpui_notify_focus(win, 1)
    ck focusedCount() == 0
    ck sendKeyToFocus("keydown", GpuiEvent(kind: gekKeyDown, key: "Down")) == 0
    ck a.lastEventSeq() == 0

  test "closing a focused window stops routing":
    let root = r.createElement("div")
    let a = r.createElement("button")
    r.appendChild(root, a)
    setFocusable(a)
    r.addEventListener(a, "keydown", proc() = discard)
    gpui_set_root_element(root)
    let win = gpui_create_window("t", 100, 100)
    discard gpui_show_window(win)
    gpui_notify_focus(win, 1)
    ck focusElement(a)
    ck sendKeyToFocus("keydown", GpuiEvent(kind: gekKeyDown, key: "a")) == 1
    discard gpui_close_window(win)
    let before = a.lastEventSeq()
    ck sendKeyToFocus("keydown", GpuiEvent(kind: gekKeyDown, key: "a")) == 0
    ck a.lastEventSeq() == before

suite "PLAT-38 — the tally":

  test "the assertion count matches the declared constant":
    # Printed for the lane, and compared against the constant. A static
    # constant alone cannot see a case that returned early before asserting.
    echo "CHECKS: ", checkCount
    check checkCount == ExpectedAssertions
