## GpuiWindow — high-level window management for the IsoNim GPUI backend.
##
## Provides:
## - Window creation with title and initial size
## - Show / close lifecycle
## - Lifecycle event callbacks (resize, focus, close)
## - Repaint request integration for reactive updates
##
## The window state machine:
##   Created -> Visible -> Closed
##
## Usage:
##   var win = createWindow("My App", 800, 600)
##   win.onResize proc(w, h: float) = echo "resized: ", w, "x", h
##   win.onFocus proc(focused: bool) = echo "focused: ", focused
##   win.onClose proc(): bool = true  # allow close
##   win.show()
##   # ... reactive updates happen, repaint is requested automatically ...
##   win.close()

import std/tables
import isonim_gpui/bindings

type
  WindowState* = enum
    ## Window lifecycle states.
    wsNotFound = 0    ## Window ID not found in registry
    wsCreated = 1     ## Window created but not yet shown
    wsVisible = 2     ## Window is visible / event loop running
    wsCloseRequested = 3  ## Close has been requested (pending)
    wsClosed = 4      ## Window is closed

  GpuiWindow* = object
    ## Handle to a GPUI window managed by the Rust shim.
    id*: uint32

# ===========================================================================
# Callback bridge — a WINDOW-ID-KEYED REGISTRY (PLAT-19)
# ===========================================================================
#
# This replaces a fixed pool of four pre-generated `cdecl` trampolines per
# callback kind. The pool had two failure modes, both reproduced before it
# was replaced, and both are now cases in
# `tests/test_window_callback_registry.nim`:
#
#   * MISROUTING AT TWO WINDOWS. `onFocus` / `onClose` chose their slot as
#     `nextWindowSlot - 1`, so registering by callback KIND
#     (`a.onResize; b.onResize; a.onFocus; b.onFocus`) made both focus
#     registrations pick slot 1. Measured: `aFocus=0 bFocus=2` — window
#     A's handler was overwritten and never ran, and B's ran for A's
#     event.
#
#   * EXHAUSTION AT THE FIFTH WINDOW, with a failure mode that depends on
#     the build. `assert nextWindowSlot < 4` aborts under the default and
#     `-d:release` profiles (both keep `assertions:on`); under `-d:danger`
#     the assert AND the bounds check are compiled out together and the
#     fifth registration is an out-of-bounds write — measured as SIGSEGV.
#
# The shape here is the one `renderer.nim` already uses for element
# events: ONE process-wide dispatcher per kind, registered once with the
# shim, plus a per-window opt-in, with the window id delivered as the
# first argument. There is no pool and no bound.

var
  resizeCallbacks: Table[uint32, proc(width, height: float)]
  focusCallbacks: Table[uint32, proc(focused: bool)]
  closeCallbacks: Table[uint32, proc(): bool]
  windowDispatchersRegistered = false

proc resizeDispatcher(windowId: uint32; w, h: cdouble) {.cdecl.} =
  let cb = resizeCallbacks.getOrDefault(windowId)
  if cb != nil: cb(w.float, h.float)

proc focusDispatcher(windowId: uint32; f: uint8) {.cdecl.} =
  let cb = focusCallbacks.getOrDefault(windowId)
  if cb != nil: cb(f != 0)

proc closeDispatcher(windowId: uint32): uint8 {.cdecl.} =
  let cb = closeCallbacks.getOrDefault(windowId)
  # No handler means "allow", matching the shim's own default.
  if cb == nil: 1'u8
  elif cb(): 1'u8
  else: 0'u8

proc ensureWindowDispatchers() =
  if not windowDispatchersRegistered:
    gpui_set_window_resize_dispatcher(resizeDispatcher)
    gpui_set_window_focus_dispatcher(focusDispatcher)
    gpui_set_window_close_dispatcher(closeDispatcher)
    windowDispatchersRegistered = true

proc windowCallbackCount*(): int =
  ## Number of windows holding at least one registered lifecycle handler.
  ## Exposed so a test can assert that destroying a window RELEASES its
  ## entries rather than merely that a later event does not arrive — the
  ## second is also true of a registry that leaks every window it ever saw.
  var ids: seq[uint32]
  for id in resizeCallbacks.keys: ids.add id
  for id in focusCallbacks.keys:
    if id notin ids: ids.add id
  for id in closeCallbacks.keys:
    if id notin ids: ids.add id
  ids.len

proc releaseWindowCallbacks(windowId: uint32) =
  resizeCallbacks.del(windowId)
  focusCallbacks.del(windowId)
  closeCallbacks.del(windowId)

# ===========================================================================
# Window API
# ===========================================================================

proc createWindow*(title: string; width, height: float): GpuiWindow =
  ## Create a new window with the given title and initial size.
  let id = gpui_create_window(title.cstring, width.cdouble, height.cdouble)
  assert id > 0, "GpuiWindow: failed to create window"
  GpuiWindow(id: id)

proc state*(win: GpuiWindow): WindowState =
  ## Get the current lifecycle state of the window.
  WindowState(gpui_window_state(win.id))

proc width*(win: GpuiWindow): float =
  ## Get the current window width.
  gpui_window_width(win.id).float

proc height*(win: GpuiWindow): float =
  ## Get the current window height.
  gpui_window_height(win.id).float

proc size*(win: GpuiWindow): tuple[width, height: float] =
  ## Get the current window size.
  (win.width, win.height)

proc show*(win: GpuiWindow): bool =
  ## Show the window (transition from Created to Visible).
  ## Returns true if the transition was successful.
  gpui_show_window(win.id) != 0

proc close*(win: GpuiWindow): bool =
  ## Request window close. If an onClose callback is registered and
  ## returns false, the close is denied.
  ## Returns true if the window was closed.
  gpui_close_window(win.id) != 0

proc destroy*(win: GpuiWindow) =
  ## Destroy the window and free its resources, including its entries in
  ## the lifecycle-callback registry. Without the release, a long-lived
  ## process that opens and closes windows accumulates one closure per
  ## window for ever — which is the leak a registry trades a pool's
  ## hard bound for, and the reason `windowCallbackCount` is exported.
  releaseWindowCallbacks(win.id)
  gpui_destroy_window(win.id)

proc onResize*(win: GpuiWindow; callback: proc(width, height: float)) =
  ## Register a callback for window resize events.
  ## Registering again for the same window replaces that window's handler
  ## and touches no other window's.
  ensureWindowDispatchers()
  resizeCallbacks[win.id] = callback
  discard gpui_on_resize_id(win.id)

proc onFocus*(win: GpuiWindow; callback: proc(focused: bool)) =
  ## Register a callback for window focus events.
  ensureWindowDispatchers()
  focusCallbacks[win.id] = callback
  discard gpui_on_focus_id(win.id)

proc onClose*(win: GpuiWindow; callback: proc(): bool) =
  ## Register a callback for window close requests.
  ## Return true from the callback to allow closing, false to prevent it.
  ensureWindowDispatchers()
  closeCallbacks[win.id] = callback
  discard gpui_on_close_id(win.id)

proc requestRepaint*() =
  ## Request a repaint of the active window. Call this after modifying
  ## the shadow tree to trigger a re-render on the next frame.
  ## Note: Tree mutation functions (appendChild, setAttribute, etc.)
  ## automatically request repaint, so this is only needed for
  ## manual/explicit repaint triggers.
  gpui_request_repaint()

proc repaintPending*(): bool =
  ## Check if a repaint has been requested (and clear the flag).
  ## This is primarily useful for testing and custom render loops.
  gpui_take_repaint_request() != 0

proc resetWindows*() =
  ## Reset all window state (for testing).
  gpui_reset_windows()
  resizeCallbacks.clear()
  focusCallbacks.clear()
  closeCallbacks.clear()
  # Re-register the dispatchers so they are always set after a reset,
  # mirroring `renderer.resetCallbacks`.
  windowDispatchersRegistered = false
  ensureWindowDispatchers()
