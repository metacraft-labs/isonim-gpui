## Raw C bindings to the gpui-nim-shim Rust cdylib.
##
## These map 1:1 to the extern "C" functions exported by
## rust/gpui-nim-shim/src/lib.rs.

type
  GpuiElement* = pointer
    ## Opaque handle to a GPUI element managed by the Rust shim.
    ## The actual layout is a Rust struct; Nim only holds a pointer to it.

const shimLib = "libgpui_nim_shim.so"  # TODO: platform-specific (.dylib on macOS)

{.push cdecl, dynlib: shimLib.}

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

type EventCallback* = proc() {.cdecl.}

proc gpui_add_event_listener*(node: GpuiElement; event: cstring; handler: EventCallback)
  {.importc: "gpui_add_event_listener".}

proc gpui_first_child*(node: GpuiElement): GpuiElement
  {.importc: "gpui_first_child".}

proc gpui_next_sibling*(node: GpuiElement): GpuiElement
  {.importc: "gpui_next_sibling".}

proc gpui_parent_node*(node: GpuiElement): GpuiElement
  {.importc: "gpui_parent_node".}

# --- Window / event loop management ---

type RootBuilderCallback* = proc(root: GpuiElement) {.cdecl.}

proc gpui_launch*(title: cstring; width, height: cdouble;
                  root_builder: RootBuilderCallback)
  {.importc: "gpui_launch".}

proc gpui_dispatch_event*(node: GpuiElement; event: cstring)
  {.importc: "gpui_dispatch_event".}

# --- Memory management ---

proc gpui_destroy_element*(handle: GpuiElement)
  {.importc: "gpui_destroy_element".}

# --- Debugging / testing ---

proc gpui_reset_tree*()
  {.importc: "gpui_reset_tree".}

proc gpui_tree_node_count*(): uint64
  {.importc: "gpui_tree_node_count".}

# --- Tree inspection (cross-renderer testing) ---

proc gpui_child_count*(node: GpuiElement): uint64
  {.importc: "gpui_child_count".}

proc gpui_get_text_content*(node: GpuiElement; buf: pointer; bufLen: uint64): uint64
  {.importc: "gpui_get_text_content".}

proc gpui_get_attribute*(node: GpuiElement; name: cstring; buf: pointer; bufLen: uint64): uint64
  {.importc: "gpui_get_attribute".}

proc gpui_nth_child*(node: GpuiElement; index: uint64): GpuiElement
  {.importc: "gpui_nth_child".}

{.pop.}
