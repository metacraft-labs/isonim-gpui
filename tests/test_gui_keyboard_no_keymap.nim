## A window that opens while the seat's keyboard has NO KEYMAP, on a real
## compositor, through the compositor's own `wl_seat`.
##
## `wl_keyboard.keymap` may announce the format `no_keymap`: the keyboard
## sends raw keycodes and the client has nothing to interpret them with. A
## wlroots compositor (sway) announces it whenever the seat has a keyboard but
## no ACTIVE one — a keyboard becomes active when it sends a key, and the seat
## loses its active keyboard when that one goes away (a keyboard unplugged
## while another stays; a virtual keyboard's client exiting). GPUI's Wayland
## client ignored that keymap, as it should, and then UNWRAPPED the keymap it
## did not have on the `modifiers` event that follows every `enter`: the
## window aborted (SIGABRT, `called Option::unwrap() on a None value` at
## `wayland/client.rs`). Measured 2026-11-23 in codetracer's GPUI plugin
## window lane: three windows of four, each opened after a `wtype` call had
## exited while a second, idle virtual keyboard stayed on the seat.
##
## The case, as the lane met it: an idle virtual keyboard (`wtype -s`) stays
## on the seat, a second one sends a key and exits — the seat now has a
## keyboard and no active one — and only then the window opens. (With the idle
## keyboard alone the window survived before the fix too, measured: sway makes the first
## keyboard it is given the active one.) Then a typist with a real keymap
## sends `Down` and the sentinel `End`. The window must survive the
## keymap-less announcement (the process reaching the assertions at all says
## so; before the fix it did not) and then receive the typist's keys, which
## carry a keymap of their own.
##
## Run with `just test-gui` (its `_run-gui-tests` runs this file after
## `test_gui.nim`), which builds the windowed shim and starts the headless
## sway.

import std/[os, osproc, times, unittest]

import isonim_gpui/bindings
import isonim_gpui/renderer

when not defined(gpuiBackend):
  {.error: "test_gui_keyboard_no_keymap must be compiled with -d:gpuiBackend " &
           "and linked against a shim built with `--features gpui-backend`: " &
           "without a window there is no wl_keyboard to announce anything to.".}

const
  ExpectedAssertions = 8

  LaunchAutoQuitMs = 60_000'u32
    ## The backstop; the case asserts the run ended on the sentinel instead.

  HolderMs = 90_000
    ## How long the keymap-less keyboard stays on the seat: past the whole
    ## run, so it is the seat's keyboard when the window binds its own.

  SettleMs = 1000
    ## After sway shows the window, before the typist's first key: the
    ## window focused.

  WindowTitle = "no keymap"

  SentinelKey = "end"

var
  keys: seq[string]
  checkCount = 0

template ck(cond: untyped) =
  inc checkCount
  check cond

suite "a keyboard without a keymap on the seat":

  test "the window opens, survives it, and still receives a typist's keys":
    require getEnv("WAYLAND_DISPLAY").len > 0
    require findExe("wtype").len > 0

    gpui_reset_tree()
    gpui_reset_windows()
    resetCallbacks()

    # A keyboard on the seat that is not active: the window is told
    # `no_keymap` when it binds its keyboard.
    let holder = startProcess("wtype", args = ["-s", $HolderMs],
                              options = {poUsePath, poStdErrToStdOut})
    sleep(1000)
    ck execCmd("wtype -k Shift_L") == 0
    sleep(500)

    let r = GpuiRenderer()
    var target: GpuiElement = nil
    proc builder(root: GpuiElement) {.cdecl.} =
      r.setStyle(root, "width", "100%")
      r.setStyle(root, "height", "100%")
      let field = r.createElement("input")
      r.setStyle(field, "width", "400px")
      r.setStyle(field, "height", "120px")
      r.appendChild(root, field)
      setFocusable(field)
      discard focusElement(field)
      target = field
      r.addEventListener(field, "keydown", proc(ev: GpuiEvent) =
        discard ev
        let k = field.lastEventKey()
        keys.add k
        if k == SentinelKey: gpui_quit())

    # The typist starts when sway shows the window, not on a clock: a key
    # sent before the window has its keyboard reaches no one, and a loaded
    # host can take seconds to map it.
    let typist = startProcess("bash", args = ["-c",
      "until swaymsg -t get_tree 2>/dev/null | grep -q '\"" & WindowTitle &
      "\"'; do sleep 0.2; done; sleep " & $(SettleMs / 1000) &
      "; wtype -s 250 -k Down; sleep 0.3; wtype -s 250 -k End"],
      options = {poUsePath, poStdErrToStdOut})
    gpui_quit_after_ms(LaunchAutoQuitMs)
    let started = epochTime()
    gpui_launch(WindowTitle.cstring, 600.0, 300.0, builder)
    let elapsedMs = int((epochTime() - started) * 1000)
    let typistRc = (if typist.running: (typist.terminate(); -1)
                    else: typist.waitForExit())
    close(typist)
    holder.terminate()
    discard holder.waitForExit()
    close(holder)

    echo "NO-KEYMAP: keys ", keys, " in ", elapsedMs, " ms"
    ck typistRc == 0
    ck target != nil
    ck keys.len == 2
    ck keys.len == 2 and keys[0] == "down"
    ck keys.len == 2 and keys[1] == SentinelKey
    ck target.deliveryCount() == keys.len
    # The run ended on the sentinel, not on the backstop.
    ck elapsedMs < int(LaunchAutoQuitMs)

  test "the assertion count matches the declared constant":
    echo "CHECKS: ", checkCount
    check checkCount == ExpectedAssertions
