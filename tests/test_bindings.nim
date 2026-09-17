## Compile-time verification of every GPUI binding.
##
## This test checks that every binding in bindings.nim has the correct
## signature (parameter types, return types, callback types). It does NOT
## link against the Rust shim — it only verifies compile-time correctness.
##
## PLAT-19: the header used to read "all 40 bindings" and the file named
## **40** of the 53 that `bindings.nim` then declared — a completeness
## claim that nothing checked, so **thirteen** bindings had never been
## verified by the file whose entire subject is verifying them (trap 6:
## the subject of a scan is a claim). Measured over HEAD's two files by
## both available counting rules — occurrence of `name(`, and occurrence
## of the bare identifier — which agree; the thirteen are
## `gpui_add_event_listener_id`, `gpui_bump_generation`,
## `gpui_free_pixels`, `gpui_free_string`, `gpui_render_cancel`,
## `gpui_render_plan_element_count`, `gpui_render_plan_json`,
## `gpui_render_submit_async`, `gpui_render_to_pixels`,
## `gpui_render_try_take`, `gpui_set_event_dispatcher`,
## `gpui_set_root_element` and `gpui_verify_render_plan`. (An earlier
## draft said 41 and twelve.) The number is no longer written down
## anywhere as a claim about coverage. The
## `static` block below READS `bindings.nim` at compile time, extracts
## every exported `proc`, and requires each name to occur in THIS file —
## so adding a binding without verifying it is a compile error, and a
## reader who deletes one from here finds out immediately.
##
## The floor matters as much as the comparison (traps 4 / 6a): an
## extractor that stopped matching would yield an empty name list, and
## "every name in the empty set occurs in this file" is a pass. The
## assertion below therefore requires a plausible population first.

import std/strutils
import isonim_gpui/bindings

const
  BindingsSrc = staticRead("../src/isonim_gpui/bindings.nim")
  SelfSrc = staticRead("test_bindings.nim")
  MinBindings = 40
    ## A floor, not the count. Deliberately below the current figure so
    ## adding an export does not fail here; exact agreement between the
    ## Rust exports and the Nim bindings is `tools/check_bindings.sh`'s
    ## job, and it is the only place that number lives.

proc exportedBindingNames(src: string): seq[string] {.compileTime.} =
  ## Every `proc <name>*` declared in bindings.nim.
  for rawLine in src.splitLines():
    let line = rawLine.strip()
    if not line.startsWith("proc "):
      continue
    let rest = line[5 .. ^1]
    var name = ""
    for c in rest:
      if c == '*':
        break
      elif c in IdentChars:
        name.add c
      else:
        name = ""
        break
    if name.len > 0 and rest.len > name.len and rest[name.len] == '*':
      result.add name

static:
  let names = exportedBindingNames(BindingsSrc)
  assert names.len >= MinBindings,
    "test_bindings: extracted only " & $names.len & " binding names from " &
    "bindings.nim (floor " & $MinBindings & "). The extractor has stopped " &
    "reading the file; every check below would pass vacuously."
  # A planted control on the extractor: a name that is definitely there.
  assert "gpui_create_element" in names,
    "test_bindings: the extractor did not find gpui_create_element"
  var unverified: seq[string]
  for n in names:
    # `SelfSrc` contains this very assertion loop, so match on a call
    # form (`name(`) rather than on the bare identifier — otherwise the
    # list literal below would satisfy itself.
    if (n & "(") notin SelfSrc:
      unverified.add n
  assert unverified.len == 0,
    "test_bindings: these bindings are declared but never exercised here: " &
    unverified.join(", ")

# ---------------------------------------------------------------------------
# Type aliases for readability
# ---------------------------------------------------------------------------
type E = GpuiElement

static:
  # --- 13 RendererBackend procs ---
  assert compiles(gpui_create_element("div".cstring))
  assert compiles(gpui_create_text_node("hello".cstring))

  var e: E
  assert compiles(gpui_append_child(e, e))
  assert compiles(gpui_insert_before(e, e, e))
  assert compiles(gpui_remove_child(e, e))
  assert compiles(gpui_set_attribute(e, "name".cstring, "val".cstring))
  assert compiles(gpui_remove_attribute(e, "name".cstring))
  assert compiles(gpui_set_text_content(e, "text".cstring))
  assert compiles(gpui_set_style(e, "color".cstring, "red".cstring))

  var cb: EventCallback
  assert compiles(gpui_add_event_listener(e, "click".cstring, cb))
  assert compiles(gpui_first_child(e))
  assert compiles(gpui_next_sibling(e))
  assert compiles(gpui_parent_node(e))

  # --- Window / event loop ---
  var rb: RootBuilderCallback
  assert compiles(gpui_launch("title".cstring, 800.0.cdouble, 600.0.cdouble, rb))
  assert compiles(gpui_dispatch_event(e, "click".cstring))

  # --- Memory management ---
  assert compiles(gpui_destroy_element(e))
  assert compiles(gpui_destroy_tree(e))

  # --- Debugging / testing ---
  assert compiles(gpui_reset_tree())
  assert compiles(gpui_tree_node_count())

  # --- Tree inspection ---
  assert compiles(gpui_child_count(e))
  assert compiles(gpui_get_text_content(e, nil, 0.uint64))
  assert compiles(gpui_get_attribute(e, "name".cstring, nil, 0.uint64))
  assert compiles(gpui_nth_child(e, 0.uint64))
  assert compiles(gpui_get_tag(e, nil, 0.uint64))
  assert compiles(gpui_get_element_kind(e))

  # --- Window management ---
  assert compiles(gpui_create_window("title".cstring, 800.0.cdouble, 600.0.cdouble))
  assert compiles(gpui_show_window(1.uint32))
  assert compiles(gpui_close_window(1.uint32))
  assert compiles(gpui_destroy_window(1.uint32))
  assert compiles(gpui_window_state(1.uint32))
  assert compiles(gpui_window_width(1.uint32))
  assert compiles(gpui_window_height(1.uint32))
  assert compiles(gpui_request_repaint())
  assert compiles(gpui_take_repaint_request())

  # --- RS-M14b: shutdown ---
  assert compiles(gpui_quit())
  assert compiles(gpui_quit_after_ms(1000.uint32))
  assert compiles(gpui_quit_requested())

  var rcb: ResizeCallback
  var fcb: FocusCallback
  var ccb: CloseCallback
  assert compiles(gpui_on_resize(1.uint32, rcb))
  assert compiles(gpui_on_focus(1.uint32, fcb))
  assert compiles(gpui_on_close(1.uint32, ccb))
  assert compiles(gpui_notify_resize(1.uint32, 800.0.cdouble, 600.0.cdouble))
  assert compiles(gpui_notify_focus(1.uint32, 1.uint8))
  assert compiles(gpui_reset_windows())

  # --- Element event dispatcher (the element-callback registry) ---
  var disp: proc(callbackId: int32) {.cdecl.}
  assert compiles(gpui_set_event_dispatcher(disp))
  assert compiles(gpui_add_event_listener_id(e, "click".cstring, 1.int32))

  # --- PLAT-19: window lifecycle dispatchers (the window registry) ---
  var wrd: WindowResizeDispatcher
  var wfd: WindowFocusDispatcher
  var wcd: WindowCloseDispatcher
  assert compiles(gpui_set_window_resize_dispatcher(wrd))
  assert compiles(gpui_set_window_focus_dispatcher(wfd))
  assert compiles(gpui_set_window_close_dispatcher(wcd))
  assert compiles(gpui_on_resize_id(1.uint32))
  assert compiles(gpui_on_focus_id(1.uint32))
  assert compiles(gpui_on_close_id(1.uint32))
  assert compiles(block:
    var x: uint8 = gpui_on_resize_id(1.uint32))
  assert compiles(block:
    var x: uint8 = gpui_on_focus_id(1.uint32))
  assert compiles(block:
    var x: uint8 = gpui_on_close_id(1.uint32))

  # --- Render plan inspection ---
  assert compiles(gpui_render_plan_json(e))
  assert compiles(gpui_free_string(nil))
  assert compiles(gpui_render_plan_element_count(e))
  assert compiles(gpui_verify_render_plan(e))
  assert compiles(block:
    var x: uint32 = gpui_render_plan_element_count(e))
  assert compiles(block:
    var x: uint8 = gpui_verify_render_plan(e))

  # --- Headless RGBA rendering (RS-M14 / EMC2-M1) ---
  var pxPtr: ptr uint8
  var pxLen: csize_t
  assert compiles(gpui_render_to_pixels(1.cuint, 1.cuint, 1.0.cfloat,
                                        addr pxPtr, addr pxLen))
  assert compiles(gpui_free_pixels(pxPtr, pxLen))
  assert compiles(gpui_set_root_element(e))
  assert compiles(gpui_render_submit_async(1.cuint, 1.cuint, 1.0.cfloat))
  assert compiles(gpui_render_try_take(1.cuint, addr pxPtr, addr pxLen))
  assert compiles(gpui_render_cancel(1.cuint))
  assert compiles(gpui_bump_generation())
  assert compiles(block:
    var x: cint = gpui_render_to_pixels(1.cuint, 1.cuint, 1.0.cfloat,
                                        addr pxPtr, addr pxLen))
  assert compiles(block:
    var x: cuint = gpui_render_submit_async(1.cuint, 1.cuint, 1.0.cfloat))
  assert compiles(block:
    var x: cint = gpui_render_try_take(1.cuint, addr pxPtr, addr pxLen))
  assert compiles(block:
    var x: uint64 = gpui_bump_generation())

  # --- Return type checks (via compiles + assignment) ---
  # Verify functions that return GpuiElement
  assert compiles(block:
    var x: E = gpui_create_element("div".cstring))
  assert compiles(block:
    var x: E = gpui_create_text_node("hello".cstring))
  assert compiles(block:
    var x: E = gpui_first_child(e))
  assert compiles(block:
    var x: E = gpui_next_sibling(e))
  assert compiles(block:
    var x: E = gpui_parent_node(e))
  assert compiles(block:
    var x: E = gpui_nth_child(e, 0.uint64))

  # Verify functions that return uint64
  assert compiles(block:
    var x: uint64 = gpui_tree_node_count())
  assert compiles(block:
    var x: uint64 = gpui_child_count(e))
  assert compiles(block:
    var x: uint64 = gpui_get_text_content(e, nil, 0.uint64))
  assert compiles(block:
    var x: uint64 = gpui_get_attribute(e, "".cstring, nil, 0.uint64))
  assert compiles(block:
    var x: uint64 = gpui_get_tag(e, nil, 0.uint64))

  # Verify functions that return uint8
  assert compiles(block:
    var x: uint8 = gpui_get_element_kind(e))
  assert compiles(block:
    var x: uint8 = gpui_show_window(1.uint32))
  assert compiles(block:
    var x: uint8 = gpui_close_window(1.uint32))
  assert compiles(block:
    var x: uint8 = gpui_window_state(1.uint32))
  assert compiles(block:
    var x: uint8 = gpui_take_repaint_request())
  assert compiles(block:
    var x: uint8 = gpui_quit_requested())

  # Verify functions that return uint32
  assert compiles(block:
    var x: uint32 = gpui_create_window("t".cstring, 1.0.cdouble, 1.0.cdouble))

  # Verify functions that return cdouble
  assert compiles(block:
    var x: cdouble = gpui_window_width(1.uint32))
  assert compiles(block:
    var x: cdouble = gpui_window_height(1.uint32))

echo "test_bindings: every binding declared in bindings.nim is verified at compile time"
