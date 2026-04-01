## Compile-time verification of all 40 GPUI bindings.
##
## This test checks that every binding in bindings.nim has the correct
## signature (parameter types, return types, callback types). It does NOT
## link against the Rust shim — it only verifies compile-time correctness.

import isonim_gpui/bindings

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

  var rcb: ResizeCallback
  var fcb: FocusCallback
  var ccb: CloseCallback
  assert compiles(gpui_on_resize(1.uint32, rcb))
  assert compiles(gpui_on_focus(1.uint32, fcb))
  assert compiles(gpui_on_close(1.uint32, ccb))
  assert compiles(gpui_notify_resize(1.uint32, 800.0.cdouble, 600.0.cdouble))
  assert compiles(gpui_notify_focus(1.uint32, 1.uint8))
  assert compiles(gpui_reset_windows())

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

  # Verify functions that return uint32
  assert compiles(block:
    var x: uint32 = gpui_create_window("t".cstring, 1.0.cdouble, 1.0.cdouble))

  # Verify functions that return cdouble
  assert compiles(block:
    var x: cdouble = gpui_window_width(1.uint32))
  assert compiles(block:
    var x: cdouble = gpui_window_height(1.uint32))

echo "test_bindings: all 40 bindings verified at compile time"
