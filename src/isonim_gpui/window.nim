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
  stateCallbacks: Table[uint32, proc()]
  activatedCallbacks: Table[uint32, proc()]
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

proc stateDispatcher(windowId: uint32) {.cdecl.} =
  let cb = stateCallbacks.getOrDefault(windowId)
  if cb != nil: cb()

proc activatedDispatcher(windowId: uint32) {.cdecl.} =
  let cb = activatedCallbacks.getOrDefault(windowId)
  if cb != nil: cb()

proc ensureWindowDispatchers() =
  if not windowDispatchersRegistered:
    gpui_set_window_resize_dispatcher(resizeDispatcher)
    gpui_set_window_focus_dispatcher(focusDispatcher)
    gpui_set_window_close_dispatcher(closeDispatcher)
    gpui_set_window_state_dispatcher(stateDispatcher)
    gpui_set_window_activated_dispatcher(activatedDispatcher)
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
  for id in stateCallbacks.keys:
    if id notin ids: ids.add id
  for id in activatedCallbacks.keys:
    if id notin ids: ids.add id
  ids.len

proc releaseWindowCallbacks(windowId: uint32) =
  resizeCallbacks.del(windowId)
  focusCallbacks.del(windowId)
  closeCallbacks.del(windowId)
  stateCallbacks.del(windowId)
  activatedCallbacks.del(windowId)

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

# ===========================================================================
# GWIN — window chrome state, identity and launch options
# (GWIN-M1)
# ===========================================================================

type
  WindowControl* = enum
    ## A window operation the platform may or may not support.
    wcMinimize, wcMaximize, wcFullscreen, wcWindowMenu

  WindowEdge* = enum
    weTop, weRight, weBottom, weLeft

proc launchWindow*(): GpuiWindow =
  ## The window `gpui_launch` is opening. Valid inside the root builder and
  ## while the event loop runs; `id == 0` at any other time. Register
  ## `onResize` / `onWindowState` on it from the root builder.
  GpuiWindow(id: gpui_launch_window_id())

proc clientDecorated*(win: GpuiWindow): bool =
  ## Whether the application is expected to draw the title bar and frame
  ## (the compositor draws nothing — GNOME/Wayland).
  gpui_window_decorations(win.id) == 1

proc tiledEdges*(win: GpuiWindow): set[WindowEdge] =
  let bits = gpui_window_tiling(win.id)
  for edge in WindowEdge:
    if (bits and (1'u8 shl ord(edge))) != 0: result.incl edge

proc supportedControls*(win: GpuiWindow): set[WindowControl] =
  let bits = gpui_window_controls(win.id)
  for c in WindowControl:
    if (bits and (1'u8 shl ord(c))) != 0: result.incl c

proc isMaximized*(win: GpuiWindow): bool =
  gpui_window_is_maximized(win.id) != 0

proc isActive*(win: GpuiWindow): bool =
  gpui_window_is_active(win.id) != 0

proc buttonLayout*(win: GpuiWindow): string =
  ## The desktop's configured button layout in GNOME `button-layout`
  ## syntax, `""` when the platform reports none. See
  ## `window_environment.windowEnvironment` for the parsed, resolved form.
  let p = gpui_window_button_layout(win.id)
  if p != nil:
    result = $cast[cstring](p)
    gpui_free_string(p)

proc onWindowState*(win: GpuiWindow; callback: proc()) =
  ## Called when any of `clientDecorated`, `tiledEdges`,
  ## `supportedControls`, `isMaximized`, `isActive` or `buttonLayout`
  ## changed — for example to swap a maximize icon for a restore icon.
  ensureWindowDispatchers()
  stateCallbacks[win.id] = callback
  discard gpui_on_window_state_id(win.id)

proc onWindowActivated*(win: GpuiWindow; callback: proc()) =
  ## Called when the window is ACTIVATED: `isActive` went from false to
  ## true — the user came back to it from another program (alt-tab, a click
  ## on it, the dock). The moment to re-read state another program may have
  ## changed while the window was in the background, as a web page does on
  ## its window `focus` event; a refresh on a clock alone leaves the window
  ## showing stale state for up to one interval after the user returns.
  ##
  ## An edge, delivered once per return: deactivation, a repeat, and any
  ## other chrome change while active (`onWindowState`'s business) do not
  ## call it. A window starts inactive, so its first active frame counts as
  ## an activation. So does a return to a window that drew nothing while the
  ## user was away (another workspace, minimized): the renderer keeps the
  ## platform's report that the window went inactive until the next frame
  ## samples it. It runs on the event loop's thread from inside the frame
  ## that sampled the change, after `onWindowState`'s callback for the same
  ## frame, so whatever it changes in the tree is drawn in that frame.
  ## Registering again replaces this window's handler; `destroy` releases it.
  ensureWindowDispatchers()
  activatedCallbacks[win.id] = callback
  discard gpui_on_window_activated_id(win.id)

proc setTitle*(win: GpuiWindow; title: string): bool =
  ## Change the window's title; the platform sees it on the next frame.
  gpui_set_window_title(win.id, title.cstring) != 0

proc addFontFile*(path: string): bool =
  ## Before `gpui_launch`: register a TrueType/OpenType font file for every
  ## launch of this process, so a `font-family` style can name its family
  ## (otherwise the platform's fallback face is drawn). False if the file
  ## cannot be read.
  gpui_add_font_file(path.cstring) != 0

proc setAppId*(appId: string) =
  ## Before `gpui_launch`: the Wayland `app_id` / X11 `WM_CLASS`. Match the
  ## application's `.desktop` file name so the desktop associates them.
  gpui_set_app_id(appId.cstring)

proc setMinWindowSize*(width, height: float) =
  ## Before `gpui_launch`: the smallest content size the app can lay out.
  gpui_set_min_window_size(width.cdouble, height.cdouble)

proc setInitialWindowState*(maximized: bool) =
  ## Before `gpui_launch`: open maximized (the launch size is then the
  ## restore size) or windowed and centred. The policy is the
  ## application's; the renderer only carries it out.
  gpui_set_initial_window_state(uint8(maximized))

# ---------------------------------------------------------------------------
# GWIN — window operations and window regions
# ---------------------------------------------------------------------------
#
# A title bar is declared, not programmed: give elements the attribute
# `windowRegionAttribute` with one of the `WindowRegion` values and the
# renderer moves, maximizes, minimizes and closes the window for presses on
# them (a drag region acts only while the application draws the decorations;
# see `clientDecorated`). These procs are for the same operations from a menu
# entry or a key binding. They act on the launched window on its next frame.

const windowRegionAttribute* = "data-window-region"

type
  WindowRegion* = enum
    wrDrag = "drag"            ## moves the window; double-click maximizes;
                               ## right-click opens the window menu
    wrNoDrag = "no-drag"       ## the application's, inside or over a drag region
    wrMinimize = "minimize"    ## a click minimizes
    wrMaximize = "maximize"    ## a click toggles maximize
    wrClose = "close"          ## a click closes, asking `onClose` first

proc minimize*(win: GpuiWindow) =
  ## Minimize the window, where the platform supports it
  ## (`wcMinimize in win.supportedControls`).
  gpui_window_minimize()

proc toggleMaximize*(win: GpuiWindow) =
  gpui_window_toggle_maximize()

proc toggleFullscreen*(win: GpuiWindow) =
  gpui_window_toggle_fullscreen()

proc requestClose*(win: GpuiWindow) =
  ## Close the window as its close button would: `onClose` decides.
  gpui_window_request_close()

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
  stateCallbacks.clear()
  activatedCallbacks.clear()
  # Re-register the dispatchers so they are always set after a reset,
  # mirroring `renderer.resetCallbacks`.
  windowDispatchersRegistered = false
  ensureWindowDispatchers()
