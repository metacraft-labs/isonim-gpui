## GWIN — the client-side window frame, on a REAL compositor.
##
## Run with `just test-gui-window-frame` (Linux), which builds the windowed
## shim and `tests/gui/window_frame_app.nim` in this repo's dev shell, then
## runs this suite in GuiAssert's, where the compositor and its session
## stack are pinned.
##
## WHAT IS DRIVEN. A private headless sway (GuiAssert's `SwayTarget`) with
## the test app floating, and the app told to ask for client-side
## decorations (`ISONIM_GPUI_DECORATIONS=client`). Sway grants that only to
## a window that is floating when it asks, so this is also the test that
## the shim asks again once the window is mapped.
##
## WHAT IS ASSERTED, and against what:
##   * the compositor's window IS the application's content: the window
##     sway reports has the size the app was last told (`onResize`) — at
##     launch, the size the app asked for — and the app's colour starts
##     exactly at its edge;
##   * the border is drawn outside that window (in the frame's band);
##   * a press reaches the app in CONTENT coordinates;
##   * each of the eight resize zones resizes on its own edge only, as sway
##     reports it, and the app is told the new content size;
##   * a fullscreen window has no frame: no band, the content is the output;
##   * with no override (sway's server-side default) there is no frame.
##
## vacuity_guard: every case first waits until the app reports a
## client-decorated window. A sway that kept server-side decorations would
## otherwise pass the "no frame" and "content equals the window" cases.
##
## MOCKS: none. A real sway, a real GPUI window through the real shim, a
## real pointer device; positions and sizes come from sway's IPC and the
## screen, never from the app's account of itself (except where the app's
## account is what is being compared).

import std/[json, os, osproc, strutils, unittest]
import gui_assert/desktop_env
import gui_assert/desktop_env/[gnome, sway]

const
  AppId = "isonim-gpui-frame-test"
  Border = (255'u8, 0'u8, 0'u8)
  Content = (0x33'u8, 0x66'u8, 0x99'u8)
  Bar = (0x00'u8, 0x80'u8, 0x00'u8)
  BarH = 32       ## the renderer's fallback title bar (`regions::FALLBACK_BAR_HEIGHT`)
  BarButton = 24  ## its buttons, 6px from the bar's ends and from each other

let appBin = getEnv("GWIN_APP")
let appLibs = getEnv("GWIN_APP_LD_LIBRARY_PATH")
if appBin.len == 0 or not fileExists(appBin):
  quit("test_gui_window_frame: GWIN_APP must name the built window_frame_app " &
       "(run `just test-gui-window-frame`)", 2)

var t: TargetEnvironment
var report: string

let targetName = getEnv("GUIENV_TARGET", "sway")

proc newTarget(): TargetEnvironment =
  ## The suite runs unchanged on every target; `GUIENV_TARGET` picks one.
  ## On sway the app is floated by rule, which is what lets sway grant it
  ## client-side decorations at all; GNOME floats every window.
  case targetName
  of "sway": newSwayTarget(1280, 800, floatingAppIds = @[AppId])
  of "gnome": newGnomeTarget(1280, 800)
  else: quit("GUIENV_TARGET must be sway or gnome, not " & targetName, 2)

proc applies(fact: string): bool =
  ## Whether a suite that needs `fact` to hold runs on this target. One that
  ## does not is reported as not applicable here, by name — never as passing.
  let t0 = newTarget()
  t0.start()
  defer: t0.stop()
  result = t0.boolFact(fact)
  if not result:
    echo "NOT APPLICABLE on ", targetName, ": ", fact, " is false"

proc events(kind: string): seq[JsonNode] =
  if not fileExists(report): return
  for line in readFile(report).splitLines():
    if line.len > 0:
      let n = parseJson(line)
      if n["event"].getStr == kind: result.add n

proc lastOf(kind: string): JsonNode =
  let all = events(kind)
  if all.len > 0: all[^1] else: newJNull()

proc launch(decorations = "client") =
  discard t.spawn(@[appBin], extraEnv = [
    ("GWIN_REPORT", report), ("GWIN_BORDER", "#ff0000ff"),
    ("GWIN_TITLEBAR_BG", "#008000ff"),
    ("ISONIM_GPUI_DECORATIONS", decorations), ("LD_LIBRARY_PATH", appLibs)])

proc contentSize(): (int, int) =
  let r = lastOf("resize")
  if r.kind == JNull: (-1, -1) else: (int(r["w"].getFloat), int(r["h"].getFloat))

proc settledWindow(csd: bool; bar = 0): EnvWindow =
  ## The window once the app reports the decoration mode `csd` and the
  ## content size it was told matches the window sway reports, less `bar`
  ## (the fallback title bar's height, where the renderer draws one).
  t.waitUntil("the app reporting csd=" & $csd & " at the compositor's size",
    proc(): bool =
      let s = lastOf("state")
      if s.kind == JNull or s["csd"].getBool != csd: return false
      let (found, w) = t.findWindow(AppId)
      found and contentSize() == (w.rect.w, w.rect.h - bar))
  result = t.window(AppId)
  discard t.captureSettled(result.rect)

suite "GWIN — the client-side frame on " & targetName:

  setup:
    t = newTarget()
    t.start()
    report = getTempDir() / ("gwin-frame-" & $getCurrentProcessId() & ".jsonl")
    removeFile(report)
    launch()

  teardown:
    t.stop()
    removeFile(report)

  test "the compositor's window is the fallback title bar and the content":
    # The app declares no title bar of its own, so the renderer draws one.
    let w = settledWindow(csd = true, bar = BarH)
    # The launch size is the window's, frame band aside.
    check (w.rect.w, w.rect.h) == (640, 400)
    let img = t.captureSettled(Rect(x: w.rect.x - 8, y: w.rect.y - 8,
                                    w: w.rect.w + 16, h: w.rect.h + 16))
    # The bar starts exactly at the window's edge, the content below it...
    check img.pixel(8 + w.rect.w div 2, 8) == Bar
    check img.pixel(8 + w.rect.w div 2, 8 + BarH - 1) == Bar
    check img.pixel(8, 8 + BarH) == Content
    check img.pixel(8 + w.rect.w - 1, 8 + w.rect.h - 1) == Content
    # ...and the border lies just outside it, in the frame's band.
    check img.pixel(7, 8 + w.rect.h div 2) == Border
    check img.pixel(8 + w.rect.w, 8 + w.rect.h div 2) == Border
    check img.pixel(8 + w.rect.w div 2, 7) == Border
    check img.pixel(8 + w.rect.w div 2, 8 + w.rect.h) == Border

  test "a press reaches the application in content coordinates":
    let w = settledWindow(csd = true, bar = BarH)
    t.click((w.rect.x + 50, w.rect.y + BarH + 40))
    t.waitUntil("the press reported", proc(): bool = events("press").len > 0)
    let p = lastOf("press")
    check int(p["x"].getFloat) == 50
    check int(p["y"].getFloat) == 40

  test "each resize zone resizes on its own edge only":
    const d = 40
    # (name, grab point relative to the window, drag, expected change in
    # x, y, w, h). The grab is 2px inside the content: within the zone's
    # reach on that side, so it lands on the window's own surface.
    type Case = tuple[name: string; gx, gy: float; dx, dy: int;
                      ex, ey, ew, eh: int]
    let cases: seq[Case] = @[
      ("right", 1.0, 0.5, d, 0, 0, 0, d, 0),
      ("bottom", 0.5, 1.0, 0, d, 0, 0, 0, d),
      ("left", 0.0, 0.5, -d, 0, -d, 0, d, 0),
      ("top", 0.5, 0.0, 0, -d, 0, -d, 0, d),
      ("bottom-right", 1.0, 1.0, d, d, 0, 0, d, d),
      ("top-left", 0.0, 0.0, -d, -d, -d, -d, d, d),
      ("top-right", 1.0, 0.0, d, -d, 0, -d, d, d),
      ("bottom-left", 0.0, 1.0, -d, d, -d, 0, d, d)]
    var w = settledWindow(csd = true, bar = BarH)
    t.placeWindow(w, 300, 200)
    w = settledWindow(csd = true, bar = BarH)
    for c in cases:
      let gx = w.rect.x + (if c.gx == 0.0: 2 elif c.gx == 1.0: w.rect.w - 3
                           else: w.rect.w div 2)
      let gy = w.rect.y + (if c.gy == 0.0: 2 elif c.gy == 1.0: w.rect.h - 3
                           else: w.rect.h div 2)
      # The holds give the compositor time to start the resize grab the app
      # requests on the press, and to finish it before the release.
      t.drag((gx, gy), (gx + c.dx, gy + c.dy), steps = 10, stepMs = 40,
             pressHoldMs = 250, releaseHoldMs = 600)
      let before = w.rect
      t.waitUntil(c.name & " resized", proc(): bool =
        t.window(AppId).rect != before)
      w = settledWindow(csd = true, bar = BarH)
      checkpoint c.name & ": " & $before & " -> " & $w.rect
      check w.rect.x - before.x == c.ex
      check w.rect.y - before.y == c.ey
      check w.rect.w - before.w == c.ew
      check w.rect.h - before.h == c.eh
      check contentSize() == (w.rect.w, w.rect.h - BarH)

  test "a press in the middle of the content does not resize":
    let w = settledWindow(csd = true, bar = BarH)
    let c = w.rect.center
    t.drag(c, c + (40, 40))
    t.waitUntil("the press reported", proc(): bool = events("press").len > 0)
    check t.window(AppId).rect == w.rect

  test "a drag on the fallback title bar moves the window":
    var w = settledWindow(csd = true, bar = BarH)
    t.placeWindow(w, 200, 150)
    w = settledWindow(csd = true, bar = BarH)
    let src = (w.rect.x + w.rect.w div 2, w.rect.y + BarH div 2)
    t.pointerMove(src[0], src[1])
    t.press()
    sleep(150)
    t.pointerMove(src[0] + 1, src[1])
    sleep(150)
    t.drag(src + (1, 0), src + (120, 70), steps = 10, stepMs = 30,
           releaseHoldMs = 250, pressHoldMs = -1)
    t.waitUntil("the window moved", proc(): bool = t.window(AppId).rect.x != w.rect.x)
    let after = t.window(AppId).rect
    # 119 or 120: the move starts on a 1px arming motion, and a compositor
    # anchors its move grab either at that motion (sway) or at the press
    # (Mutter), which is a difference between them, not between runs.
    check after.x - w.rect.x in [119, 120]
    check (after.y - w.rect.y, after.w, after.h) == (70, w.rect.w, w.rect.h)

  test "the fallback title bar's close button is where the desktop wants it":
    # Two layouts, close alone on each side; a click at the other end closes
    # nothing, a click on the button does. The button's centre is 6 + 12px
    # from its end of the bar.
    let ends = [("close:", true), (":close", false)]
    for i, (layout, onLeft) in ends:
      if i > 0:
        # The previous layout's app closed; a fresh one for this layout.
        removeFile(report)
        launch()
      let w = settledWindow(csd = true, bar = BarH)
      t.configure("buttonLayout", layout)
      t.waitUntil("the app told the layout " & layout, proc(): bool =
        let s = lastOf("state")
        s.kind != JNull and s["layout"].getStr == layout)
      sleep(300)
      let left = (w.rect.x + 6 + BarButton div 2, w.rect.y + BarH div 2)
      let right = (w.rect.x + w.rect.w - 6 - BarButton div 2, w.rect.y + BarH div 2)
      t.click(if onLeft: right else: left)
      sleep(400)
      check t.findWindow(AppId)[0]
      t.click(if onLeft: left else: right)
      t.waitUntil("closed by the " & (if onLeft: "left" else: "right") & " button",
        proc(): bool = not t.findWindow(AppId)[0])

  test "a fullscreen window has no frame and no title bar":
    discard settledWindow(csd = true, bar = BarH)
    t.setFullscreen(t.window(AppId), true)
    t.waitUntil("the app told it is tiled on every edge", proc(): bool =
      let s = lastOf("state")
      s.kind != JNull and s["tiling"].getInt == 15)
    t.waitUntil("the content filling the output", proc(): bool =
      contentSize() == (1280, 800) and t.window(AppId).rect == Rect(x: 0, y: 0, w: 1280, h: 800))
    let w = t.window(AppId)
    check w.rect == Rect(x: 0, y: 0, w: 1280, h: 800)
    let img = t.captureSettled(Rect(x: 0, y: 0, w: 1280, h: 800))
    # No band, no border: the content reaches the output's edges.
    check img.pixel(0, 400) == Content
    check img.pixel(1279, 400) == Content
    check img.pixel(640, 0) == Content

if applies("decorations.serverAvailable"):
 suite "GWIN — no frame where the compositor decorates":

  setup:
    t = newTarget()
    t.start()
    report = getTempDir() / ("gwin-frame-" & $getCurrentProcessId() & ".jsonl")
    removeFile(report)
    launch(decorations = "")

  teardown:
    t.stop()
    removeFile(report)

  test "server-side decorations: no band and no border":
    let w = settledWindow(csd = false)
    let img = t.captureSettled(Rect(x: w.rect.x - 8, y: w.rect.y - 8,
                                    w: w.rect.w + 16, h: w.rect.h + 16))
    check img.pixel(8, 8) == Content
    check img.pixel(7, 8 + w.rect.h div 2) != Border
    check img.pixel(8 + w.rect.w div 2, 7) != Border
    t.click((w.rect.x + 50, w.rect.y + 40))
    t.waitUntil("the press reported", proc(): bool = events("press").len > 0)
    check int(lastOf("press")["x"].getFloat) == 50

# ---------------------------------------------------------------------------
# Window regions and window operations. The app's title bar (`GWIN_REGIONS`,
# `window_frame_app.buildTitleBar`): a drag strip 40px high across the top, a
# `no-drag` child of it at x=100, a listener drawn over it at x=220, and the
# window buttons the environment supports at its right end.
# ---------------------------------------------------------------------------



const
  TitleBarH = 40
  NoDragX = 100
  ListenerX = 220
  ButtonSize = 30
  ButtonGap = 6
  ButtonMargin = 10

var app: Process

proc launchRegions(decorations = "client") =
  app = t.spawn(@[appBin], extraEnv = [
    ("GWIN_REPORT", report), ("GWIN_REGIONS", "1"),
    ("ISONIM_GPUI_DECORATIONS", decorations), ("LD_LIBRARY_PATH", appLibs)])

proc buttonOrder(): seq[string] =
  ## The buttons the app last drew, in order: "@[maximize, close]".
  let b = lastOf("buttons")
  if b.kind == JNull: return
  for name in b["order"].getStr.strip(chars = {'@', '[', ']'}).split(", "):
    if name.len > 0: result.add name

proc buttonCenter(w: EnvWindow; name: string): (int, int) =
  let order = buttonOrder()
  let i = order.find(name)
  doAssert i >= 0, "the app drew no " & name & " button: " & $order
  let fromRight = ButtonMargin + (order.len - 1 - i) * (ButtonSize + ButtonGap)
  (w.rect.x + w.rect.w - fromRight - ButtonSize div 2, w.rect.y + TitleBarH div 2)

proc moveDrag(src: (int, int); delta: (int, int)) =
  ## A title-bar drag. The move starts on the FIRST motion after the press
  ## (so that a double-click can still maximize), and the compositor's move
  ## follows the pointer from there: that first motion is 1px, and the
  ## window travels `delta - (1, 0)`.
  t.pointerMove(src[0], src[1])
  t.press()
  sleep(150)
  t.pointerMove(src[0] + 1, src[1])
  sleep(150)
  t.drag(src + (1, 0), src + delta, steps = 10, stepMs = 30,
         releaseHoldMs = 250, pressHoldMs = -1)

suite "GWIN — window regions on " & targetName:

  setup:
    t = newTarget()
    t.start()
    report = getTempDir() / ("gwin-regions-" & $getCurrentProcessId() & ".jsonl")
    removeFile(report)
    launchRegions()

  teardown:
    t.stop()
    removeFile(report)

  test "a drag on the title bar moves the window, and only that":
    var w = settledWindow(csd = true)
    t.placeWindow(w, 200, 150)
    w = settledWindow(csd = true)
    moveDrag((w.rect.x + 40, w.rect.y + TitleBarH div 2), (120, 70))
    t.waitUntil("the window moved", proc(): bool = t.window(AppId).rect.x != w.rect.x)
    let after = t.window(AppId).rect
    # See "a drag on the fallback title bar": 119 or 120 by the compositor.
    check after.x - w.rect.x in [119, 120]
    check (after.y - w.rect.y, after.w, after.h) == (70, w.rect.w, w.rect.h)

  test "an application with its own title bar gets no fallback one":
    let w = settledWindow(csd = true)
    let img = t.captureSettled(w.rect)
    check img.pixel(40, 5) == (0x22'u8, 0x44'u8, 0x66'u8)   # the app's own strip

  test "a drag that starts on a no-drag child of the title bar does not move it":
    let w = settledWindow(csd = true)
    moveDrag((w.rect.x + NoDragX + 10, w.rect.y + TitleBarH div 2), (120, 70))
    sleep(300)
    check t.window(AppId).rect == w.rect

  test "a drag that starts on a listener drawn over the title bar is the listener's":
    let w = settledWindow(csd = true)
    moveDrag((w.rect.x + ListenerX + 10, w.rect.y + TitleBarH div 2), (120, 70))
    t.waitUntil("the listener's press", proc(): bool =
      events("listener-press").len > 0)
    check t.window(AppId).rect == w.rect

  test "a drag below the title bar does not move the window":
    let w = settledWindow(csd = true)
    moveDrag((w.rect.x + 300, w.rect.y + 200), (120, 70))
    t.waitUntil("the press reported", proc(): bool = events("press").len > 0)
    check t.window(AppId).rect == w.rect

  test "the buttons are the ones the environment supports, where it wants them":
    discard settledWindow(csd = true)
    let advertised = t.fact("window.capabilities").value.split(',')
    proc expected(layout: string): seq[string] =
      # The configured layout, left side then right, less what the
      # compositor does not advertise (close is always available).
      for name in layout.replace(":", ",").split(','):
        if name in ["minimize", "maximize", "close"] and name notin result and
           (name == "close" or name in advertised):
          result.add name
    for layout in [":minimize,maximize,close", "close,maximize:minimize"]:
      t.configure("buttonLayout", layout)
      t.waitUntil("the buttons for " & layout, proc(): bool =
        buttonOrder() == expected(layout))
      checkpoint layout & " -> " & $buttonOrder()
      check "close" in buttonOrder()
      check ("minimize" in buttonOrder()) == ("minimize" in advertised)

  test "a double-click on the title bar maximizes where the compositor allows it":
    let w = settledWindow(csd = true)
    t.doubleClick((w.rect.x + 40, w.rect.y + TitleBarH div 2))
    if t.boolFact("window.canMaximize"):
      t.waitUntil("maximized", proc(): bool =
        let s = lastOf("state")
        s.kind != JNull and s["maximized"].getBool)
      # A maximized window has no frame: the compositor's window is the
      # content. A second double-click on its title bar restores it.
      t.waitUntil("the content filling the maximized window", proc(): bool =
        let m = t.window(AppId)
        wsMaximized in m.states and contentSize() == (m.rect.w, m.rect.h))
      let m = t.window(AppId)
      sleep(800)   # past the double-click interval: a new gesture, not clicks 3 and 4
      t.doubleClick((m.rect.x + 40, m.rect.y + TitleBarH div 2))
      # Restored to the size it had, not grown by the frame's band.
      t.waitUntil("restored to " & $w.rect.w & "x" & $w.rect.h, proc(): bool =
        let s = lastOf("state")
        let r = t.window(AppId).rect
        s.kind != JNull and not s["maximized"].getBool and
          (r.w, r.h) == (w.rect.w, w.rect.h))
    else:
      sleep(500)
      check t.window(AppId).rect == w.rect
      check not lastOf("state")["maximized"].getBool

  test "the minimize button minimizes where the compositor allows it":
    let w = settledWindow(csd = true)
    if not t.boolFact("window.canMinimize"):
      check "minimize" notin buttonOrder()
    else:
      t.configure("buttonLayout", ":minimize,maximize,close")
      t.waitUntil("a minimize button", proc(): bool = "minimize" in buttonOrder())
      t.click(buttonCenter(t.window(AppId), "minimize"))
      if t.boolFact("window.reportsMinimized"):
        t.waitUntil("minimized", proc(): bool = wsMinimized in t.window(AppId).states)

  test "the close button asks onClose, and closes when it agrees":
    let w = settledWindow(csd = true)
    t.focusWindow(w)
    t.keys("v")   # refuse the next close
    sleep(200)
    t.click(buttonCenter(w, "close"))
    t.waitUntil("the close refused", proc(): bool =
      events("close-refused").len > 0)
    check t.findWindow(AppId)[0]
    t.click(buttonCenter(t.window(AppId), "close"))
    t.waitUntil("the app exited", proc(): bool = not app.running)
    check app.peekExitCode == 0
    check events("close-allowed").len == 1
    check not t.findWindow(AppId)[0]

  test "keys toggle fullscreen and close through the imperative operations":
    let w = settledWindow(csd = true)
    t.focusWindow(w)
    t.requireFacts("window.canFullscreen")
    if t.boolFact("window.canFullscreen"):
      t.keys("f")
      t.waitUntil("fullscreen", proc(): bool =
        t.window(AppId).rect == Rect(x: 0, y: 0, w: 1280, h: 800))
      t.keys("f")
      t.waitUntil("restored", proc(): bool =
        let r = t.window(AppId).rect
        r.w == w.rect.w and r.h == w.rect.h)
    t.keys("q")
    t.waitUntil("the app exited", proc(): bool = not app.running)
    check app.peekExitCode == 0
    check events("close-allowed").len == 1

if applies("decorations.serverAvailable"):
 suite "GWIN — window regions where the compositor decorates":

  setup:
    t = newTarget()
    t.start()
    report = getTempDir() / ("gwin-regions-" & $getCurrentProcessId() & ".jsonl")
    removeFile(report)
    launchRegions(decorations = "")

  teardown:
    t.stop()
    removeFile(report)

  test "a drag region is inert, and a window button still acts":
    let w = settledWindow(csd = false)
    moveDrag((w.rect.x + 40, w.rect.y + TitleBarH div 2), (120, 70))
    sleep(300)
    check t.window(AppId).rect == w.rect
    t.click(buttonCenter(w, "close"))
    t.waitUntil("the app exited", proc(): bool = not app.running)
    check app.peekExitCode == 0
