## PLAT-19 — window lifecycle callbacks must be routed by WINDOW ID,
## not by a fixed pool of pre-generated `cdecl` trampolines.
##
## Build & run:
##   LD_LIBRARY_PATH=rust/target/debug nim c -r tests/test_window_callback_registry.nim
##
## MOCKS: none. Every case drives the real Rust shim through the real
## `extern "C"` boundary (`gpui_create_window`, `gpui_on_*`,
## `gpui_notify_*`, `gpui_close_window`) and observes the effect in a
## Nim closure the shim called back into. There is no substitute for the
## shim here and none is used: the defect these cases were written
## against lives exactly at that boundary, in which trampoline the Nim
## side hands to `gpui_on_focus`.
##
## ---------------------------------------------------------------------
## WHAT WAS MEASURED BEFORE THE REPAIR (2026-09-15, this host)
## ---------------------------------------------------------------------
## `window.nim` carried `array[4, proc …]` per callback kind plus four
## hand-written `cdecl` trampolines each, and `onFocus` / `onClose`
## picked their slot as `nextWindowSlot - 1` — "the slot the last
## `allocWindowSlot` handed out". Two failure modes, both reproduced
## before anything was changed, by `probe_window_pool.nim`:
##
##   1. MISROUTING, at TWO windows — not sixteen. Registration grouped
##      by callback kind (`a.onResize; b.onResize; a.onFocus; b.onFocus`
##      — a shell wiring resize for every window up front) makes both
##      `onFocus` calls choose slot `nextWindowSlot - 1 == 1`. The second
##      assignment overwrites the first, and window A's registered
##      trampoline is the one now holding B's closure:
##
##        per-window order  aResize=1 bResize=1 aFocus=1 bFocus=1   (ok)
##        per-kind    order  aResize=1 bResize=1 aFocus=0 bFocus=2   <-- A DROPPED
##
##   2. EXHAUSTION, at the FIFTH window, and the failure mode depends on
##      the build — which is why the build is named here (§12b). Same
##      source, same host, three profiles, registering `onResize` for
##      six windows:
##
##        | build        | assertions | boundChecks | 5th registration     |
##        | ------------ | ---------- | ----------- | -------------------- |
##        | (debug)      | true       | true        | AssertionDefect, abort |
##        | -d:release   | true       | true        | AssertionDefect, abort |
##        | -d:danger    | FALSE      | FALSE       | **SIGSEGV**          |
##
##      `-d:danger` removes the `assert nextWindowSlot < 4` AND the
##      bounds check on `resizeCallbacks[4]` together, so the bound that
##      is documented as "maximum number of concurrent windows (4)
##      exceeded" becomes an out-of-bounds write in the profile a
##      shipping build is most likely to use.
##
## The cases below are the cases that reproduced those two states. They
## were red first; the registry is what makes them green.

import std/[tables, strutils]
import unittest
import isonim_gpui/bindings
import isonim_gpui/window

# --- §4c counted assertions. `ck` is a TEMPLATE, never a proc: trap 13 ---
var asserted = 0

template ck(condition: untyped) =
  inc asserted
  check condition

template ckEq(a, b: untyped) =
  inc asserted
  check a == b

template expectCount(expected: int) =
  if asserted != expected:
    checkpoint("assertion count is " & $asserted & ", expected " & $expected)
  check asserted == expected

# ---------------------------------------------------------------------
# 1. Misrouting — the two-window case, in BOTH registration orders
# ---------------------------------------------------------------------

suite "PLAT-19 window callbacks: routing is per window, not per slot":

  setup:
    resetWindows()

  test "two windows, callbacks registered per WINDOW, each fires for its own events":
    let a = createWindow("A", 100, 100)
    let b = createWindow("B", 100, 100)
    var aResize, bResize, aFocus, bFocus = 0
    a.onResize(proc(w, h: float) = inc aResize)
    a.onFocus(proc(f: bool) = inc aFocus)
    b.onResize(proc(w, h: float) = inc bResize)
    b.onFocus(proc(f: bool) = inc bFocus)

    gpui_notify_focus(a.id, 1)
    gpui_notify_focus(b.id, 1)
    gpui_notify_resize(a.id, 10, 10)
    gpui_notify_resize(b.id, 20, 20)

    ckEq aResize, 1
    ckEq bResize, 1
    ckEq aFocus, 1
    ckEq bFocus, 1

  test "two windows, callbacks registered per KIND, each fires for its own events":
    # This is the order that lost window A's focus handler outright.
    let a = createWindow("A", 100, 100)
    let b = createWindow("B", 100, 100)
    var aResize, bResize, aFocus, bFocus = 0
    a.onResize(proc(w, h: float) = inc aResize)
    b.onResize(proc(w, h: float) = inc bResize)
    a.onFocus(proc(f: bool) = inc aFocus)
    b.onFocus(proc(f: bool) = inc bFocus)

    gpui_notify_focus(a.id, 1)
    gpui_notify_focus(b.id, 1)
    gpui_notify_resize(a.id, 10, 10)
    gpui_notify_resize(b.id, 20, 20)

    ckEq aResize, 1
    ckEq bResize, 1
    ckEq aFocus, 1
    ckEq bFocus, 1

  test "onClose is routed per window: denying on one does not deny on the other":
    let a = createWindow("A", 100, 100)
    let b = createWindow("B", 100, 100)
    discard a.show()
    discard b.show()
    a.onClose(proc(): bool = false) # A refuses to close
    b.onClose(proc(): bool = true)  # B allows it

    ck not a.close()
    ckEq a.state, wsVisible
    ck b.close()
    ckEq b.state, wsClosed

# ---------------------------------------------------------------------
# 2. The bound itself — more callbacks than any fixed pool has slots
# ---------------------------------------------------------------------

const WindowCount = 20
  ## Deliberately above the sixteen PLAT-19 names and far above the four
  ## the pool actually had. 20 windows x 3 kinds = 60 registrations.

type Counters = ref seq[int]

# Closures made INSIDE a `for` body share one environment in Nim, so
# `let idx = i; proc() = inc c[idx]` gives every window the last index.
# A maker proc gives each closure its own environment — the same reason
# `test_renderer.nim`'s 150-callback case has a `makeHandler`. Written
# the other way the suite is green on a registry that routes everything
# to one window, which is the failure it exists to detect.
proc makeResizeHandler(c: Counters; idx: int): proc(w, h: float) =
  result = proc(w, h: float) = inc c[][idx]

proc makeFocusHandler(c: Counters; idx: int): proc(focused: bool) =
  result = proc(focused: bool) = inc c[][idx]

proc makeCloseHandler(c: Counters; idx: int): proc(): bool =
  result = proc(): bool =
    inc c[][idx]
    true

suite "PLAT-19 window callbacks: no fixed pool bound":

  setup:
    resetWindows()

  test "20 windows x 3 callback kinds all register and all route to their own window":
    var wins: seq[GpuiWindow]
    for i in 0 ..< WindowCount:
      wins.add createWindow("W" & $i, 100, 100)
      discard wins[i].show()

    let resized = Counters(new(seq[int])); resized[] = newSeq[int](WindowCount)
    let focused = Counters(new(seq[int])); focused[] = newSeq[int](WindowCount)
    let closed = Counters(new(seq[int])); closed[] = newSeq[int](WindowCount)

    # Registered grouped by kind — the order that broke at two windows.
    for i in 0 ..< WindowCount:
      wins[i].onResize(makeResizeHandler(resized, i))
    for i in 0 ..< WindowCount:
      wins[i].onFocus(makeFocusHandler(focused, i))
    for i in 0 ..< WindowCount:
      wins[i].onClose(makeCloseHandler(closed, i))

    for i in 0 ..< WindowCount:
      gpui_notify_resize(wins[i].id, float(100 + i), float(200 + i))
      gpui_notify_focus(wins[i].id, 1)
      ck wins[i].close()

    for i in 0 ..< WindowCount:
      ckEq resized[][i], 1
      ckEq focused[][i], 1
      ckEq closed[][i], 1

    # The resize payload reached the right window too, so "it fired" is
    # not satisfied by a dispatcher that fires everything with garbage.
    for i in 0 ..< WindowCount:
      ckEq wins[i].width, float(100 + i)
      ckEq wins[i].height, float(200 + i)

  test "NEGATIVE CONTROL: notifying ONE window moves exactly that window's counters":
    # §7a. Without this, "all 20 fired" is satisfied by a dispatcher that
    # broadcasts every event to every registered closure — which is the
    # other way a registry can be wrong, and the positive case above
    # cannot tell the two apart.
    var wins: seq[GpuiWindow]
    for i in 0 ..< WindowCount:
      wins.add createWindow("W" & $i, 100, 100)

    let resized = Counters(new(seq[int])); resized[] = newSeq[int](WindowCount)
    for i in 0 ..< WindowCount:
      wins[i].onResize(makeResizeHandler(resized, i))

    const Chosen = 7
    gpui_notify_resize(wins[Chosen].id, 640, 480)

    for i in 0 ..< WindowCount:
      ckEq resized[][i], (if i == Chosen: 1 else: 0)

  test "NEGATIVE CONTROL: a window with no handler registered fires nothing, and an unknown id is inert":
    let a = createWindow("A", 100, 100)
    let b = createWindow("B", 100, 100)
    var aResize = 0
    a.onResize(proc(w, h: float) = inc aResize)
    # b deliberately has no handler.

    gpui_notify_resize(b.id, 1, 1)
    ckEq aResize, 0

    # An id no window owns must not reach any registered closure.
    gpui_notify_resize(a.id + b.id + 1000, 1, 1)
    ckEq aResize, 0

    gpui_notify_resize(a.id, 1, 1)
    ckEq aResize, 1

  test "re-registering replaces that window's handler and leaves its neighbours alone":
    let a = createWindow("A", 100, 100)
    let b = createWindow("B", 100, 100)
    var first, second, bHits = 0
    a.onResize(proc(w, h: float) = inc first)
    b.onResize(proc(w, h: float) = inc bHits)
    a.onResize(proc(w, h: float) = inc second)

    gpui_notify_resize(a.id, 1, 1)
    gpui_notify_resize(b.id, 1, 1)

    ckEq first, 0
    ckEq second, 1
    ckEq bHits, 1

  test "destroying a window releases its registry entries":
    let a = createWindow("A", 100, 100)
    var hits = 0
    a.onResize(proc(w, h: float) = inc hits)
    ckEq windowCallbackCount(), 1
    a.destroy()
    ckEq windowCallbackCount(), 0
    # And the shim no longer knows the window either.
    gpui_notify_resize(a.id, 1, 1)
    ckEq hits, 0

  test "resetWindows clears the registry":
    let a = createWindow("A", 100, 100)
    a.onResize(proc(w, h: float) = discard)
    a.onFocus(proc(f: bool) = discard)
    a.onClose(proc(): bool = true)
    ckEq windowCallbackCount(), 1
    resetWindows()
    ckEq windowCallbackCount(), 0

# ---------------------------------------------------------------------
# 3. The count is the fingerprint (§4b / §4c)
# ---------------------------------------------------------------------

suite "PLAT-19 window callbacks: assertion count":
  test "every assertion above ran":
    # 4 + 4 + 4  (routing suite)
    # + (20*3 closes = 20 ck) + 60 + 40 = 120
    # + 20 (single-notify control)
    # + 3 (no-handler control)
    # + 3 (re-registration)
    # + 3 (destroy)
    # + 2 (reset)
    # = 163 before this case
    expectCount(163)
