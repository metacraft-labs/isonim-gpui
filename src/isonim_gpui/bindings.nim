## Raw C bindings to the gpui-nim-shim Rust cdylib.
##
## These map 1:1 to the extern "C" functions exported by
## rust/gpui-nim-shim/src/lib.rs.
##
## 40 exported symbols total:
##   13 RendererBackend + window management + tree inspection + utilities

type
  GpuiElement* = pointer
    ## Opaque handle to a GPUI element managed by the Rust shim.
    ## The actual layout is a Rust struct; Nim only holds a pointer to it.

when defined(macosx):
  const shimLib = "libgpui_nim_shim.dylib"
elif defined(windows):
  const shimLib = "gpui_nim_shim.dll"
else:
  const shimLib = "libgpui_nim_shim.so"

# --- Callback types ---

type EventCallback* = proc() {.cdecl.}
type RootBuilderCallback* = proc(root: GpuiElement) {.cdecl.}
type ResizeCallback* = proc(width: cdouble; height: cdouble) {.cdecl.}
type FocusCallback* = proc(focused: uint8) {.cdecl.}
type CloseCallback* = proc(): uint8 {.cdecl.}

{.push cdecl, dynlib: shimLib.}

# ===========================================================================
# 13 RendererBackend procs
# ===========================================================================

proc gpui_create_element*(tag: cstring): GpuiElement
  {.importc: "gpui_create_element".}

proc gpui_create_text_node*(text: cstring): GpuiElement
  {.importc: "gpui_create_text_node".}

proc gpui_append_child*(parent, child: GpuiElement)
  {.importc: "gpui_append_child".}

proc gpui_insert_before*(parent, child, reference: GpuiElement)
  {.importc: "gpui_insert_before".}

proc gpui_remove_child*(parent, child: GpuiElement)
  {.importc: "gpui_remove_child".}

proc gpui_set_attribute*(node: GpuiElement; name, value: cstring)
  {.importc: "gpui_set_attribute".}

proc gpui_remove_attribute*(node: GpuiElement; name: cstring)
  {.importc: "gpui_remove_attribute".}

proc gpui_set_text_content*(node: GpuiElement; text: cstring)
  {.importc: "gpui_set_text_content".}

proc gpui_set_style*(node: GpuiElement; prop, value: cstring)
  {.importc: "gpui_set_style".}

proc gpui_add_event_listener*(node: GpuiElement; event: cstring; handler: EventCallback)
  {.importc: "gpui_add_event_listener".}

proc gpui_add_event_listener_id*(node: GpuiElement; event: cstring; callbackId: int32)
  {.importc: "gpui_add_event_listener_id".}

type EventDispatcherCallback* = proc(callbackId: int32) {.cdecl.}

proc gpui_set_event_dispatcher*(dispatcher: EventDispatcherCallback)
  {.importc: "gpui_set_event_dispatcher".}

proc gpui_first_child*(node: GpuiElement): GpuiElement
  {.importc: "gpui_first_child".}

proc gpui_next_sibling*(node: GpuiElement): GpuiElement
  {.importc: "gpui_next_sibling".}

proc gpui_parent_node*(node: GpuiElement): GpuiElement
  {.importc: "gpui_parent_node".}

# ===========================================================================
# Window / event loop management
# ===========================================================================

proc gpui_launch*(title: cstring; width, height: cdouble;
                  root_builder: RootBuilderCallback)
  {.importc: "gpui_launch".}

proc gpui_dispatch_event*(node: GpuiElement; event: cstring)
  {.importc: "gpui_dispatch_event".}

# ===========================================================================
# Memory management
# ===========================================================================

proc gpui_destroy_element*(handle: GpuiElement)
  {.importc: "gpui_destroy_element".}

proc gpui_destroy_tree*(handle: GpuiElement)
  {.importc: "gpui_destroy_tree".}

# ===========================================================================
# Debugging / testing
# ===========================================================================

proc gpui_reset_tree*()
  {.importc: "gpui_reset_tree".}

proc gpui_tree_node_count*(): uint64
  {.importc: "gpui_tree_node_count".}

# ===========================================================================
# Tree inspection (cross-renderer testing)
# ===========================================================================

proc gpui_child_count*(node: GpuiElement): uint64
  {.importc: "gpui_child_count".}

proc gpui_get_text_content*(node: GpuiElement; buf: pointer; bufLen: uint64): uint64
  {.importc: "gpui_get_text_content".}

proc gpui_get_attribute*(node: GpuiElement; name: cstring; buf: pointer; bufLen: uint64): uint64
  {.importc: "gpui_get_attribute".}

proc gpui_nth_child*(node: GpuiElement; index: uint64): GpuiElement
  {.importc: "gpui_nth_child".}

proc gpui_get_tag*(node: GpuiElement; buf: pointer; bufLen: uint64): uint64
  {.importc: "gpui_get_tag".}

proc gpui_get_element_kind*(node: GpuiElement): uint8
  {.importc: "gpui_get_element_kind".}

# ===========================================================================
# Window management
# ===========================================================================

proc gpui_create_window*(title: cstring; width, height: cdouble): uint32
  {.importc: "gpui_create_window".}

proc gpui_show_window*(window_id: uint32): uint8
  {.importc: "gpui_show_window".}

proc gpui_close_window*(window_id: uint32): uint8
  {.importc: "gpui_close_window".}

proc gpui_destroy_window*(window_id: uint32)
  {.importc: "gpui_destroy_window".}

proc gpui_window_state*(window_id: uint32): uint8
  {.importc: "gpui_window_state".}

proc gpui_window_width*(window_id: uint32): cdouble
  {.importc: "gpui_window_width".}

proc gpui_window_height*(window_id: uint32): cdouble
  {.importc: "gpui_window_height".}

proc gpui_request_repaint*()
  {.importc: "gpui_request_repaint".}

proc gpui_take_repaint_request*(): uint8
  {.importc: "gpui_take_repaint_request".}

proc gpui_on_resize*(window_id: uint32; callback: ResizeCallback)
  {.importc: "gpui_on_resize".}

proc gpui_on_focus*(window_id: uint32; callback: FocusCallback)
  {.importc: "gpui_on_focus".}

proc gpui_on_close*(window_id: uint32; callback: CloseCallback)
  {.importc: "gpui_on_close".}

proc gpui_notify_resize*(window_id: uint32; width, height: cdouble)
  {.importc: "gpui_notify_resize".}

proc gpui_notify_focus*(window_id: uint32; focused: uint8)
  {.importc: "gpui_notify_focus".}

proc gpui_reset_windows*()
  {.importc: "gpui_reset_windows".}

# ===========================================================================
# Render plan inspection
# ===========================================================================

proc gpui_render_plan_json*(root: GpuiElement): pointer
  {.importc: "gpui_render_plan_json".}

proc gpui_free_string*(p: pointer)
  {.importc: "gpui_free_string".}

proc gpui_render_plan_element_count*(root: GpuiElement): uint32
  {.importc: "gpui_render_plan_element_count".}

proc gpui_verify_render_plan*(root: GpuiElement): uint8
  {.importc: "gpui_verify_render_plan".}

{.pop.}
