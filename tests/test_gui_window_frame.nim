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

import std/[json, os, strutils, unittest]
import gui_assert/desktop_env
import gui_assert/desktop_env/sway

const
  AppId = "isonim-gpui-frame-test"
  Border = (255'u8, 0'u8, 0'u8)
  Content = (0x33'u8, 0x66'u8, 0x99'u8)

let appBin = getEnv("GWIN_APP")
let appLibs = getEnv("GWIN_APP_LD_LIBRARY_PATH")
if appBin.len == 0 or not fileExists(appBin):
  quit("test_gui_window_frame: GWIN_APP must name the built window_frame_app " &
       "(run `just test-gui-window-frame`)", 2)

var t: SwayTarget
var report: string

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
    ("ISONIM_GPUI_DECORATIONS", decorations), ("LD_LIBRARY_PATH", appLibs)])

proc contentSize(): (int, int) =
  let r = lastOf("resize")
  if r.kind == JNull: (-1, -1) else: (int(r["w"].getFloat), int(r["h"].getFloat))

proc settledWindow(csd: bool): EnvWindow =
  ## The window once the app reports the decoration mode `csd` and the
  ## content size it was told matches the window sway reports.
  t.waitUntil("the app reporting csd=" & $csd & " at the compositor's size",
    proc(): bool =
      let s = lastOf("state")
      if s.kind == JNull or s["csd"].getBool != csd: return false
      let (found, w) = t.findWindow(AppId)
      found and contentSize() == (w.rect.w, w.rect.h))
  result = t.window(AppId)
  discard t.captureSettled(result.rect)

suite "GWIN — the client-side frame on sway":

  setup:
    t = newSwayTarget(1280, 800, floatingAppIds = @[AppId])
    t.start()
    report = getTempDir() / ("gwin-frame-" & $getCurrentProcessId() & ".jsonl")
    removeFile(report)
    launch()

  teardown:
    t.stop()
    removeFile(report)

  test "the compositor's window is the application's content":
    let w = settledWindow(csd = true)
    # The launch size is the CONTENT's: the frame's band is added around it.
    check (w.rect.w, w.rect.h) == (640, 400)
    let img = t.captureSettled(Rect(x: w.rect.x - 8, y: w.rect.y - 8,
                                    w: w.rect.w + 16, h: w.rect.h + 16))
    # The content's colour starts exactly at the window's edge...
    check img.pixel(8, 8) == Content
    check img.pixel(8 + w.rect.w - 1, 8 + w.rect.h - 1) == Content
    # ...and the border lies just outside it, in the frame's band.
    check img.pixel(7, 8 + w.rect.h div 2) == Border
    check img.pixel(8 + w.rect.w, 8 + w.rect.h div 2) == Border
    check img.pixel(8 + w.rect.w div 2, 7) == Border
    check img.pixel(8 + w.rect.w div 2, 8 + w.rect.h) == Border

  test "a press reaches the application in content coordinates":
    let w = settledWindow(csd = true)
    t.click((w.rect.x + 50, w.rect.y + 40))
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
    var w = settledWindow(csd = true)
    t.command("[app_id=" & AppId & "] move position 300 200")
    w = settledWindow(csd = true)
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
      w = settledWindow(csd = true)
      checkpoint c.name & ": " & $before & " -> " & $w.rect
      check w.rect.x - before.x == c.ex
      check w.rect.y - before.y == c.ey
      check w.rect.w - before.w == c.ew
      check w.rect.h - before.h == c.eh
      check contentSize() == (w.rect.w, w.rect.h)

  test "a press in the middle of the content does not resize":
    let w = settledWindow(csd = true)
    let c = w.rect.center
    t.drag(c, c + (40, 40))
    t.waitUntil("the press reported", proc(): bool = events("press").len > 0)
    check t.window(AppId).rect == w.rect

  test "a fullscreen window has no frame":
    discard settledWindow(csd = true)
    t.command("[app_id=" & AppId & "] fullscreen enable")
    t.waitUntil("the app told it is tiled on every edge", proc(): bool =
      let s = lastOf("state")
      s.kind != JNull and s["tiling"].getInt == 15)
    t.waitUntil("the content filling the output", proc(): bool =
      contentSize() == (1280, 800))
    let w = t.window(AppId)
    check w.rect == Rect(x: 0, y: 0, w: 1280, h: 800)
    let img = t.captureSettled(Rect(x: 0, y: 0, w: 1280, h: 800))
    # No band, no border: the content reaches the output's edges.
    check img.pixel(0, 400) == Content
    check img.pixel(1279, 400) == Content
    check img.pixel(640, 0) == Content

suite "GWIN — no frame where the compositor decorates":

  setup:
    t = newSwayTarget(1280, 800, floatingAppIds = @[AppId])
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
