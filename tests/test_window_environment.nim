## GWIN-M1 — window chrome state, the window-environment helper and the
## launch options, through the real shim.
##
## Build & run:
##   LD_LIBRARY_PATH=rust/target/debug nim c -r tests/test_window_environment.nim
##
## WHAT IS COVERED
##
##   * `resolveWindowEnvironment`, the placement policy, over fixed inputs:
##     a configured GNOME layout on either side, GNOME's own default
##     (`appmenu:close`), a compositor that cannot minimize (sway), a
##     platform with no reported layout (the platform default, and macOS's),
##     unknown and repeated names, and a malformed layout.
##   * The chrome-state path end to end: `gpui_notify_window_state` records
##     a state in the shim's window registry, the `GpuiWindow` accessors and
##     `windowEnvironment` read it back across the C boundary, and
##     `onWindowState` fires once per CHANGE and not for a repeat — the
##     discrimination that makes it usable for redraws.
##   * `onWindowActivated` fires on the inactive-to-active EDGE only — once
##     per return to the window, not for a repeat, a deactivation or a
##     maximize while active — for the window it happened to, after the same
##     sample's `onWindowState`, with `isActive` already true — and on a
##     return from a window that drew no frame while the user was away, which
##     only the platform's activation report (`gpui_notify_window_activation`)
##     saw leave.
##   * `launchWindow` is id 0 outside `gpui_launch`, and the title setter
##     reports whether the window exists.
##
## WHAT IS NOT — and where it is
##
##   The SAMPLING of a live GPUI window (`gpui_app::sample_chrome_state`,
##   the resize notification from `NimRootView::render`) needs a compositor.
##   It is exercised by the GuiAssert window scenarios of GWIN-M2 onward,
##   not here.
##
## MOCKS: none. `gpui_notify_window_state` is the shim's own bridging entry
## point (the counterpart of `gpui_notify_resize`, which the window-registry
## suite drives the same way); it writes the same registry field the render
## path writes, so the read-back and the dispatch tested here are the
## production ones.

import std/unittest
import isonim_gpui/[bindings, window, window_environment]

const allControls = {wcMinimize, wcMaximize, wcFullscreen, wcWindowMenu}

suite "GWIN — resolveWindowEnvironment":

  test "a configured right-side layout is followed, in order":
    let env = resolveWindowEnvironment("linux-wayland", "GNOME", true,
                                       allControls, ":minimize,maximize,close")
    check env.placementSource == psPortal
    check env.preferredLeft.len == 0
    check env.preferredRight == @[wbMinimize, wbMaximize, wbClose]

  test "a configured left-side layout puts the buttons on the left":
    let env = resolveWindowEnvironment("linux-wayland", "GNOME", true,
                                       allControls, "close,minimize,maximize:")
    check env.preferredLeft == @[wbClose, wbMinimize, wbMaximize]
    check env.preferredRight.len == 0

  test "GNOME's default layout (appmenu:close) is close only, on the right":
    let env = resolveWindowEnvironment("linux-wayland", "GNOME", true,
                                       allControls, "appmenu:close")
    check env.preferredLeft.len == 0
    check env.preferredRight == @[wbClose]

  test "an unsupported control is dropped from the placement (sway: no minimize)":
    let env = resolveWindowEnvironment("linux-wayland", "sway", true,
                                       {wcMaximize, wcFullscreen},
                                       ":minimize,maximize,close")
    check wbMinimize notin env.supported
    check env.supported == {wbMaximize, wbClose}
    check env.preferredRight == @[wbMaximize, wbClose]

  test "close is always supported, even with no controls reported":
    let env = resolveWindowEnvironment("linux-wayland", "", true, {},
                                       ":minimize,maximize,close")
    check env.supported == {wbClose}
    check env.preferredRight == @[wbClose]

  test "no reported layout falls back to the platform default, and says so":
    let linux = resolveWindowEnvironment("linux-x11", "", true, allControls, "")
    check linux.placementSource == psPlatformDefault
    check linux.preferredRight == @[wbMinimize, wbMaximize, wbClose]
    let mac = resolveWindowEnvironment("macos", "", false, allControls, "")
    check mac.preferredLeft == @[wbClose, wbMinimize, wbMaximize]
    check mac.preferredRight.len == 0
    check mac.titleBarHeight == 28.0

  test "unknown names are skipped and repeats keep their first position":
    let parsed = parseButtonLayout("icon,close:spacer,close,maximize,maximize")
    check parsed.ok
    check parsed.left == @[wbClose]
    check parsed.right == @[wbMaximize]

  test "a layout without a colon is not a layout":
    let env = resolveWindowEnvironment("linux-wayland", "", true, allControls,
                                       "close,minimize")
    check env.placementSource == psPlatformDefault
    check env.preferredRight == @[wbMinimize, wbMaximize, wbClose]

  test "an explicitly empty layout is honoured: no buttons":
    let env = resolveWindowEnvironment("linux-wayland", "", true, allControls, ":")
    check env.placementSource == psPortal
    check env.preferredLeft.len == 0 and env.preferredRight.len == 0

suite "GWIN — chrome state through the shim":

  setup:
    resetWindows()

  test "a new window reads as server-decorated with every control":
    let win = createWindow("fresh", 100, 100)
    check not win.clientDecorated
    check win.supportedControls == allControls
    check win.tiledEdges == {}
    check win.buttonLayout == ""

  test "a reported state reads back through every accessor":
    let win = createWindow("csd", 100, 100)
    gpui_notify_window_state(win.id, 1, 1 or 8, 2, 1, 1, "close:maximize")
    check win.clientDecorated
    check win.tiledEdges == {weTop, weLeft}
    check win.supportedControls == {wcMaximize}
    check win.isMaximized
    check win.isActive
    check win.buttonLayout == "close:maximize"
    let env = windowEnvironment(win)
    check env.clientDecorated
    check env.preferredLeft == @[wbClose]
    check env.preferredRight == @[wbMaximize]

  test "onWindowState fires once per change, never for a repeat":
    let win = createWindow("watched", 100, 100)
    let other = createWindow("unwatched", 100, 100)
    var calls = 0
    win.onWindowState(proc() = inc calls)
    gpui_notify_window_state(win.id, 1, 0, 15, 0, 1, ":close")
    check calls == 1
    gpui_notify_window_state(win.id, 1, 0, 15, 0, 1, ":close")
    check calls == 1
    gpui_notify_window_state(win.id, 1, 0, 15, 1, 1, ":close")  # maximized
    check calls == 2
    # Another window's change is not this window's callback.
    gpui_notify_window_state(other.id, 1, 0, 15, 1, 1, ":close")
    check calls == 2

  test "the handler can read the new state from inside the callback":
    let win = createWindow("reader", 100, 100)
    var seen = false
    win.onWindowState(proc() = seen = win.isMaximized)
    gpui_notify_window_state(win.id, 1, 0, 15, 1, 0, "")
    check seen

  test "destroying a window releases its state callback":
    let win = createWindow("gone", 100, 100)
    win.onWindowState(proc() = discard)
    check windowCallbackCount() == 1
    win.destroy()
    check windowCallbackCount() == 0

suite "window activation — the inactive-to-active edge":

  setup:
    resetWindows()

  test "onWindowActivated fires on each return, never while still active":
    let win = createWindow("returned-to", 100, 100)
    var calls = 0
    win.onWindowActivated(proc() = inc calls)
    # A window starts inactive; an inactive sample is no activation.
    gpui_notify_window_state(win.id, 1, 0, 15, 0, 0, ":close")
    check calls == 0
    # The user switches to it.
    gpui_notify_window_state(win.id, 1, 0, 15, 0, 1, ":close")
    check calls == 1
    # Still active: a repeat, and a maximize while active, are not returns.
    gpui_notify_window_state(win.id, 1, 0, 15, 0, 1, ":close")
    gpui_notify_window_state(win.id, 1, 0, 15, 1, 1, ":close")
    check calls == 1
    # Away (another program focused) and back again.
    gpui_notify_window_state(win.id, 1, 0, 15, 1, 0, ":close")
    check calls == 1
    gpui_notify_window_state(win.id, 1, 0, 15, 1, 1, ":close")
    check calls == 2

  test "the edge reaches only the window it happened to":
    let a = createWindow("a", 100, 100)
    let b = createWindow("b", 100, 100)
    var aCalls, bCalls = 0
    a.onWindowActivated(proc() = inc aCalls)
    b.onWindowActivated(proc() = inc bCalls)
    gpui_notify_window_state(b.id, 1, 0, 15, 0, 1, "")
    check aCalls == 0
    check bCalls == 1

  test "the handler reads the window as active, after onWindowState ran":
    let win = createWindow("ordered", 100, 100)
    var order: seq[string]
    win.onWindowState(proc() = order.add "state")
    win.onWindowActivated(proc() =
      order.add(if win.isActive: "activated-active" else: "activated-inactive"))
    gpui_notify_window_state(win.id, 1, 0, 15, 0, 1, "")
    check order == @["state", "activated-active"]

  test "a return the frame sampler never saw leave is still one activation":
    let win = createWindow("hidden-away", 100, 100)
    var calls = 0
    win.onWindowActivated(proc() = inc calls)
    gpui_notify_window_state(win.id, 1, 0, 15, 0, 1, ":close")
    check calls == 1
    # Another workspace: the platform reports the window inactive and back,
    # and no frame runs in between, so no sample sees it inactive.
    check gpui_notify_window_activation(win.id, 0) == 1
    check gpui_notify_window_activation(win.id, 1) == 1
    check calls == 1
    # The first frame after the return samples it active again: a return.
    gpui_notify_window_state(win.id, 1, 0, 15, 0, 1, ":close")
    check calls == 2
    check win.isActive
    # And only one: the next frame is not another.
    gpui_notify_window_state(win.id, 1, 0, 15, 0, 1, ":close")
    check calls == 2

  test "destroying a window releases its activation callback":
    let win = createWindow("gone", 100, 100)
    win.onWindowActivated(proc() = discard)
    check windowCallbackCount() == 1
    win.destroy()
    check windowCallbackCount() == 0

suite "GWIN — identity and launch options":

  setup:
    resetWindows()

  test "launchWindow is id 0 outside gpui_launch":
    check launchWindow().id == 0

  test "setTitle reports whether the window exists":
    let win = createWindow("old", 100, 100)
    check win.setTitle("new")
    check not GpuiWindow(id: 9999).setTitle("nobody")

  test "the launch options are accepted before a launch":
    # Stored for the next `gpui_launch` and reset by it; the Rust unit
    # tests (`window::tests::test_launch_options_are_consumed_once`) read
    # them back. Here: the C entry points exist and take these values.
    setAppId("codetracer")
    setMinWindowSize(1050, 600)
    setInitialWindowState(maximized = true)
    resetWindows()
