## Raw C bindings to the gpui-nim-shim Rust cdylib.
##
## These map 1:1 to the extern "C" functions exported by
## rust/gpui-nim-shim/src/lib.rs.
##
## The symbol count is NOT written down here. It is asserted, against the
## Rust source, by `tools/check_bindings.sh` (which both sides run in CI)
## and by `tests/test_bindings.nim`, so a number in a comment cannot
## drift away from the table it describes. See PLAT-19.
##
##   13 RendererBackend + window management + tree inspection + utilities

import std/os

type
  GpuiElement* = pointer
    ## Opaque handle to a GPUI element managed by the Rust shim.
    ## The actual layout is a Rust struct; Nim only holds a pointer to it.

const shimTargetDir = currentSourcePath().parentDir.parentDir.parentDir /
  "rust" / "target" / "debug"

when defined(macosx):
  const localShimLib = shimTargetDir / "libgpui_nim_shim.dylib"
  when fileExists(localShimLib):
    const shimLib = localShimLib
  else:
    const shimLib = "libgpui_nim_shim.dylib"
elif defined(windows):
  const localShimLib = shimTargetDir / "gpui_nim_shim.dll"
  when fileExists(localShimLib):
    const shimLib = localShimLib
  else:
    const shimLib = "gpui_nim_shim.dll"
else:
  const localShimLib = shimTargetDir / "libgpui_nim_shim.so"
  when fileExists(localShimLib):
    const shimLib = localShimLib
  else:
    const shimLib = "libgpui_nim_shim.so"

# --- The event payload (PLAT-38) ---

type
  GpuiEventPayload* {.bycopy.} = object
    ## **What a dispatched event hands its listener.** The Nim mirror of
    ## `rust/gpui-nim-shim/src/input.rs`'s `#[repr(C)] GpuiEventPayload`.
    ##
    ## Field ORDER and TYPES are the ABI; `tests/test_input_focus.nim`
    ## asserts `sizeof`, `alignof` and every `offsetof` against what
    ## `gpui_event_payload_layout` reports from the RUST side — not against
    ## a literal both sides could be wrong about together. A layout that
    ## drifted would not fail to link; it would deliver a different key,
    ## which is the failure mode
    ## `codetracer-specs/Testing/Verification-Harness-Traps.md` §25 names
    ## one level up.
    kind*: uint32          ## one of `GpuiEventKind*`
    modifiers*: uint32     ## `GpuiMod*` bits
    key*: cstring          ## base key name, or nil when the event has no key
    repeat*: uint8         ## 1 for an auto-repeat
    reserved*: array[7, uint8]

const
  GpuiEventOther* = 0'u32
  GpuiEventKeyDown* = 1'u32
  GpuiEventKeyUp* = 2'u32

  GpuiModControl* = 1'u32 shl 0
  GpuiModAlt* = 1'u32 shl 1
  GpuiModShift* = 1'u32 shl 2
  GpuiModPlatform* = 1'u32 shl 3
  GpuiModFunction* = 1'u32 shl 4

# --- Callback types ---

type EventCallback* = proc(payload: ptr GpuiEventPayload) {.cdecl.}
  ## **PLAT-38 widened this from `proc() {.cdecl.}`.** A listener that takes
  ## no argument can be told which handler it is and nothing else, which is
  ## why `PLAT21-VG1` says a key cannot be delivered through this renderer at
  ## all. `payload` is nil for an event that carries nothing.
type RootBuilderCallback* = proc(root: GpuiElement) {.cdecl.}
type ResizeCallback* = proc(width: cdouble; height: cdouble) {.cdecl.}
type FocusCallback* = proc(focused: uint8) {.cdecl.}
type CloseCallback* = proc(): uint8 {.cdecl.}

# PLAT-19 — window-id-carrying dispatchers. One per event kind,
# registered process-wide; the id is the first argument, so the Nim side
# keeps an id-keyed registry instead of a fixed pool of trampolines.
type WindowResizeDispatcher* = proc(windowId: uint32; width: cdouble;
                                     height: cdouble) {.cdecl.}
type WindowFocusDispatcher* = proc(windowId: uint32; focused: uint8) {.cdecl.}
type WindowCloseDispatcher* = proc(windowId: uint32): uint8 {.cdecl.}

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

type EventDispatcherCallback* = proc(callbackId: int32;
                                     payload: ptr GpuiEventPayload) {.cdecl.}
  ## **PLAT-38 widened this too.** See `EventCallback`.

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
# PLAT-38 — payload-carrying delivery, and element focus
# ===========================================================================
#
# `PLAT21-VG1` (a key cannot be delivered) and `PLAT21-VG3` (no element
# focus) are the two gaps this block closes. Read
# `rust/gpui-nim-shim/src/input.rs`'s module docs before changing any of it:
# three of the decisions below (the payload being a base key plus a BITMASK,
# focus being a per-node FLAG rather than one remembered id, and the store
# recording a delivery BEFORE the callback runs) are each there to keep a
# published assertion able to fail.

proc gpui_event_payload_layout*(field: uint32): uint64
  {.importc: "gpui_event_payload_layout".}
  ## The Rust compiler's own answer for `GpuiEventPayload`'s layout:
  ## 0 = size, 1 = align, 2..5 = offset of `kind`, `modifiers`, `key`,
  ## `repeat`. See `GpuiEventPayload`.

proc gpui_dispatch_event_with*(node: GpuiElement; event: cstring;
                               payload: ptr GpuiEventPayload): uint32
  {.importc: "gpui_dispatch_event_with".}
  ## Dispatch with a payload. **Returns the number of listeners reached** —
  ## which is what separates "a key that reached nothing" from "a key that
  ## reached everything", two states a `void` dispatch cannot tell apart.

proc gpui_dispatch_key_to_focus*(event: cstring;
                                 payload: ptr GpuiEventPayload): uint32
  {.importc: "gpui_dispatch_key_to_focus".}
  ## Route a key to whatever element holds focus. Answers 0 when nothing
  ## holds focus AND when no WINDOW holds focus — the second is the negative
  ## twin PLAT-38 asks for.

# --- the element store, read back ---

proc gpui_last_event_key*(node: GpuiElement; buf: pointer; bufLen: uint64): uint64
  {.importc: "gpui_last_event_key".}

proc gpui_last_event_name*(node: GpuiElement; buf: pointer; bufLen: uint64): uint64
  {.importc: "gpui_last_event_name".}

proc gpui_last_event_modifiers*(node: GpuiElement): uint32
  {.importc: "gpui_last_event_modifiers".}

proc gpui_last_event_kind*(node: GpuiElement): uint32
  {.importc: "gpui_last_event_kind".}

proc gpui_last_event_repeat*(node: GpuiElement): uint8
  {.importc: "gpui_last_event_repeat".}

proc gpui_last_event_seq*(node: GpuiElement): uint64
  {.importc: "gpui_last_event_seq".}
  ## 0 means nothing ever arrived. That is the only way to distinguish
  ## "nothing arrived" from "the same thing arrived again".

proc gpui_event_delivery_count*(node: GpuiElement): uint64
  {.importc: "gpui_event_delivery_count".}

# --- focus ---

proc gpui_set_focusable*(node: GpuiElement; focusable: uint8)
  {.importc: "gpui_set_focusable".}

proc gpui_is_focusable*(node: GpuiElement): uint8
  {.importc: "gpui_is_focusable".}

proc gpui_focus_element*(node: GpuiElement): uint8
  {.importc: "gpui_focus_element".}
  ## 1 if it took focus, 0 if REFUSED (not focusable, or outside an active
  ## trap). A refusal is what makes a focus trap mean something.

proc gpui_blur_element*(node: GpuiElement)
  {.importc: "gpui_blur_element".}

proc gpui_is_focused*(node: GpuiElement): uint8
  {.importc: "gpui_is_focused".}

proc gpui_focused_element*(): GpuiElement
  {.importc: "gpui_focused_element".}

proc gpui_focused_count*(): uint64
  {.importc: "gpui_focused_count".}
  ## **The focus partition law's instrument.** Counts every focused node in
  ## the store, not the one the caller just focused.

proc gpui_set_focus_trap*(node: GpuiElement; trap: uint8): uint8
  {.importc: "gpui_set_focus_trap".}

proc gpui_focus_trap_element*(): GpuiElement
  {.importc: "gpui_focus_trap_element".}

proc gpui_focusable_count*(): uint64
  {.importc: "gpui_focusable_count".}

proc gpui_focusable_at*(index: uint64): GpuiElement
  {.importc: "gpui_focusable_at".}
  ## The declared focus ORDER, readable from the Rust side. PLAT-35 filed
  ## `PLAT35-VG4` because the order was *"declared by the leaf renderer and
  ## enforced by nothing"*; an order only the caller can see is one no gate
  ## can check.

proc gpui_focus_next*(): uint8
  {.importc: "gpui_focus_next".}

proc gpui_focus_prev*(): uint8
  {.importc: "gpui_focus_prev".}

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

proc gpui_node_id*(node: GpuiElement): uint64
  {.importc: "gpui_node_id".}
  ## Stable identity of the shadow-tree node this handle refers to.
  ##
  ## NOT redundant with the handle pointer: `gpui_nth_child`,
  ## `gpui_first_child` and `gpui_parent_node` each return a FRESHLY
  ## allocated handle, so two handles to the same node are different
  ## pointers. Compare this instead — `renderer.sameNode` wraps it.
  ## 0 means "no node" (null handle or null id).

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

# --- RS-M14b: shutdown ---
#
# `gpui_launch` built with `--features gpui-backend` blocks inside GPUI's
# platform event loop. These are the only way to make it return; without
# them a windowed test cannot terminate and has to be killed. See the
# "Shutdown" section of `rust/gpui-nim-shim/src/window.rs`.

proc gpui_quit*()
  {.importc: "gpui_quit".}
  ## Ask a running GPUI event loop to stop, so `gpui_launch` returns.
  ## Safe from any thread — a windowed test calls it from a watcher
  ## thread once it has the evidence it came for.

proc gpui_quit_after_ms*(ms: uint32)
  {.importc: "gpui_quit_after_ms".}
  ## Arm an automatic quit `ms` after the loop starts (0 disarms).
  ## Call BEFORE `gpui_launch`. This is the backstop that keeps a
  ## windowed test bounded even when its watcher never fires.

proc gpui_quit_requested*(): uint8
  {.importc: "gpui_quit_requested".}
  ## 1 if a quit is latched and not yet consumed. Observation only.

proc gpui_on_resize*(window_id: uint32; callback: ResizeCallback)
  {.importc: "gpui_on_resize".}

proc gpui_on_focus*(window_id: uint32; callback: FocusCallback)
  {.importc: "gpui_on_focus".}

proc gpui_on_close*(window_id: uint32; callback: CloseCallback)
  {.importc: "gpui_on_close".}

# --- PLAT-19: id-carrying window lifecycle dispatch ---

proc gpui_set_window_resize_dispatcher*(dispatcher: WindowResizeDispatcher)
  {.importc: "gpui_set_window_resize_dispatcher".}

proc gpui_set_window_focus_dispatcher*(dispatcher: WindowFocusDispatcher)
  {.importc: "gpui_set_window_focus_dispatcher".}

proc gpui_set_window_close_dispatcher*(dispatcher: WindowCloseDispatcher)
  {.importc: "gpui_set_window_close_dispatcher".}

proc gpui_on_resize_id*(window_id: uint32): uint8
  {.importc: "gpui_on_resize_id".}

proc gpui_on_focus_id*(window_id: uint32): uint8
  {.importc: "gpui_on_focus_id".}

proc gpui_on_close_id*(window_id: uint32): uint8
  {.importc: "gpui_on_close_id".}

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

# --- RS-M14 Phase 2: headless RGBA rendering via Zed's HeadlessAppContext ---
#
# Only exported when the shim is built with `--features gpui-headless`.
# Returns 0 on success and writes the RGBA8888 pixel-buffer ptr +
# byte count to the caller's out pointers. The buffer is owned by
# the shim and MUST be released via `gpui_free_pixels(p, len)`.
# On error returns a non-zero code and writes (nil, 0) to the
# out pointers, so naive cleanup paths remain safe.
#
# Error codes (matching `gpui_headless::ErrorCode`):
#   1 - InvalidArgs (zero dims, scale <= 0, null out-params)
#   2 - RendererUnavailable (Linux: current_headless_renderer returned None)
#   3 - WindowOpenFailed
#   4 - CaptureFailed (Window::render_to_image error)
#   5 - SizeMismatch
#   6 - Panic (caught across the FFI boundary)

proc gpui_render_to_pixels*(width: cuint; height: cuint; scale: cfloat;
                             outPtr: ptr ptr uint8;
                             outLen: ptr csize_t): cint
  {.importc: "gpui_render_to_pixels".}

proc gpui_free_pixels*(p: ptr uint8; len: csize_t)
  {.importc: "gpui_free_pixels".}

# RS-M14 Phase 2: callers that build the shadow tree without going through
# `gpui_launch` (e.g. the streaming adapter in isonim-render-serve) must
# call this before `gpui_render_to_pixels` so the headless renderer finds
# the correct tree root. Passing nil resets to NULL.
proc gpui_set_root_element*(handle: GpuiElement)
  {.importc: "gpui_set_root_element".}

# --- EMC2-M1: dedicated GPUI render thread (async API) ---
#
# The async API moves ``Window::render_to_image`` + the deferred-draw pump
# off the bridge's async ``frameLoop`` thread onto a dedicated worker
# thread that lazily constructs and reuses ``HeadlessAppContext`` across
# frames. The bridge submits frame N+1 immediately and polls for frame
# N-1 on the next tick; if the result is not yet ready, the bridge emits
# the previous frame again (smoother than blocking on the synchronous
# ~41 ms ``render_to_image`` wait).
#
# Token semantics:
#   * ``gpui_render_submit_async`` returns 0 on submission failure and a
#     non-zero monotonic token otherwise.
#   * ``gpui_render_try_take`` consumes a token on Ready / Failed /
#     UnknownToken; leaves it alive on Pending.
#
# Try-take return codes:
#   *  0 (TakeReady)       — render complete; ptr/len carry RGBA bytes.
#                            Release via ``gpui_free_pixels``.
#   *  1 (TakePending)     — still in flight; ptr/len are (nil, 0).
#   * -1 to -6             — negation of the per-render ``ErrorCode``
#                            (1=InvalidArgs … 6=Panic). Token consumed.
#   * -100 (UnknownToken)  — token never submitted, already taken, or
#                            expired. Token consumed.

proc gpui_render_submit_async*(width: cuint; height: cuint;
                                scale: cfloat): cuint
  {.importc: "gpui_render_submit_async".}

proc gpui_render_try_take*(token: cuint;
                            outPtr: ptr ptr uint8;
                            outLen: ptr csize_t): cint
  {.importc: "gpui_render_try_take".}

proc gpui_render_cancel*(token: cuint)
  {.importc: "gpui_render_cancel".}

# ERV-M3: story-generation guard.
#
# Bump the shared story / tree generation counter inside the shim
# BEFORE mutating the GPUI shadow tree (i.e. inside the Nim
# adapter's ``select-story`` handler). Render requests submitted
# at an earlier generation are dropped by ``gpui_render_try_take``
# with the new ``GpuiRenderTakeStale`` (2) sentinel — preventing
# the prior story's pixels from being painted after the switch.
#
# Returns the new (post-bump) generation value. No handle parameter:
# the shim's render worker is a single global so the counter is
# process-wide.
proc gpui_bump_generation*(): uint64
  {.importc: "gpui_bump_generation".}

{.pop.}

const
  GpuiRenderTakeReady*        = cint(0)
  GpuiRenderTakePending*      = cint(1)
  GpuiRenderTakeStale*        = cint(2)
    ## ERV-M3: the in-flight render was submitted at an older
    ## generation (a story-switch bumped the counter after
    ## ``gpui_render_submit_async`` snapshotted it). Bytes were
    ## freed by the shim; the token is consumed. Treat like
    ## Pending — submit a fresh render against the current tree.
  GpuiRenderTakeUnknownToken* = cint(-100)
