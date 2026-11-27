## test_gui_conic.nim — the window paints a two-stop conic gradient the way a
## browser does.
##
## `background-image: conic-gradient(<c1> 0 <p>%, <c2> <p>% 100%)` on a round
## element (`border-radius: 50%`) is the one gradient the shim paints
## (`rust/gpui-nim-shim/src/conic.rs`; CSS Images Level 4 §3.3,
## https://www.w3.org/TR/css-images-4/#conic-gradients): a sector of `c1`
## from twelve o'clock clockwise to `p`% of the turn, over the rest of the
## disc in `c2`. This suite draws five such discs — red over blue at 0, 16,
## 50, 99 and 100 %, 40px each, in a row with a 10px gap, on white — in a
## REAL window on a headless sway, reads the window's pixels back
## (`wayland-capture-frame.sh` and `grim`, as `test_gui_borders.nim` does)
## and compares them with Chromium's rendering of the same elements
## (`tests/fixtures/conic/browser-strip.ppm`):
##
##   * per disc, the red-dominant pixels number the browser's count within
##     2 % of the disc's area: the sector covers the same share of the turn;
##   * every pixel the browser draws in one pure colour, with its eight
##     neighbours the same, is that colour within 8 per channel here: the
##     sector lies on the same side of twelve o'clock and ends at the same
##     angle (a counter-clockwise sector, or one from another start, fails
##     here even where the count agrees). Only the antialiased edges, where
##     two rasterisers legitimately differ, are left out: the circle's rim
##     (the browser's own mixed pixels) and the pixels a stop line crosses
##     (`nearStopLine`: Chromium samples a hard stop at each pixel's centre,
##     the shim covers it by area).
##
## The reference is a FIXTURE, not a browser run, because this lane has no
## browser: the fixture was taken once by `tools/conic-reference/capture.sh`
## from `tools/conic-reference/page.html` (the same five elements), whose
## header names the Chromium that produced it. No mocks: the real shim, a
## real window, a real compositor.
##
## Run by `just test-gui-conic` (headless sway, the windowed shim).

import unittest
import std/[math, os, osproc, strformat, strutils]

import isonim_gpui/renderer
import isonim_gpui/bindings

const
  RootBg = "#ffffff"
  First = "#ff0000"
  Rest = "#0000ff"
  DiscPx = 40
  GapPx = 10
  Percents = [0, 16, 50, 99, 100]
  CountTolerance = 0.02
    ## Of the disc's area (pi r^2): the red-dominant pixel count's tolerance.
  PureTolerance = 8
    ## Per channel, at a pixel the browser draws in one pure colour.

  CaptureTimeoutSec = 30
  CaptureBackstopMs = 60_000'u32
  WatcherTickMs = 50
  WatcherMaxTicks = 1200

type
  Frame = object
    w, h: int
    pixels: string

proc readPpm(path: string): Frame =
  let raw = readFile(path)
  var fields: seq[string] = @[]
  var i = 0
  while fields.len < 4 and i < raw.len:
    while i < raw.len and raw[i] in Whitespace: inc i
    if i < raw.len and raw[i] == '#':
      while i < raw.len and raw[i] != '\n': inc i
      continue
    var j = i
    while j < raw.len and raw[j] notin Whitespace: inc j
    if j > i: fields.add raw[i ..< j]
    i = j
  doAssert fields.len == 4 and fields[0] == "P6" and fields[3] == "255"
  inc i
  result.w = parseInt(fields[1])
  result.h = parseInt(fields[2])
  result.pixels = raw[i ..< i + result.w * result.h * 3]

proc rgbAt(f: Frame; x, y: int): (int, int, int) =
  let k = (y * f.w + x) * 3
  (int(uint8(f.pixels[k])), int(uint8(f.pixels[k + 1])),
   int(uint8(f.pixels[k + 2])))

proc isFirst(c: (int, int, int)): bool = c[0] > 128 and c[2] < 128

proc discLeft(k: int): int = k * (DiscPx + GapPx)

proc nearStopLine(x, y: int): bool =
  ## Whether pixel (`x`, `y`) of the strip is crossed by one of its disc's
  ## two stop lines: the ray from the centre to twelve o'clock and the ray
  ## at `p`% of the turn, clockwise. Chromium colours a hard stop by the
  ## angle at each pixel's centre, so its stop lines are drawn without
  ## antialiasing and the pixels beside them read as pure; the shim fills
  ## the sector as an antialiased path, so a pixel the line crosses is
  ## mixed by its coverage. Where the 99 % disc's 1 % sliver is narrower
  ## than a pixel, the two disagree along its whole length. Those pixels
  ## are rasterisation, not geometry, and are left out like the circle's
  ## antialiased rim; every other pixel of the disc is compared.
  let k = x div (DiscPx + GapPx)
  if k >= Percents.len: return false
  let p = Percents[k]
  if p <= 0 or p >= 100: return false
  let r = DiscPx / 2
  let vx = float(x) + 0.5 - (float(discLeft(k)) + r)
  let vy = float(y) + 0.5 - r
  for turn in [0.0, float(p) / 100.0]:
    let a = turn * 2.0 * PI
    let (dx, dy) = (sin(a), -cos(a))   # clockwise from the top, y down
    let t = vx * dx + vy * dy
    let dist = if t < 0: hypot(vx, vy) else: hypot(vx - t * dx, vy - t * dy)
    if dist < 0.75: return true   # within a pixel's half-diagonal
  false

proc buildScene(root: GpuiElement) {.cdecl.} =
  let r = GpuiRenderer()
  r.setStyle(root, "background-color", RootBg)
  r.setStyle(root, "width", "100%")
  r.setStyle(root, "height", "100%")
  r.setStyle(root, "flex-direction", "column")
  let row = r.createElement("div")
  r.setStyle(row, "flex-direction", "row")
  r.setStyle(row, "gap", $GapPx & "px")
  for p in Percents:
    let disc = r.createElement("div")
    r.setStyle(disc, "width", $DiscPx & "px")
    r.setStyle(disc, "height", $DiscPx & "px")
    r.setStyle(disc, "flex-shrink", "0")
    r.setStyle(disc, "border-radius", "50%")
    r.setStyle(disc, "background-image",
               &"conic-gradient({First} 0 {p}%, {Rest} {p}% 100%)")
    r.appendChild(row, disc)
  r.appendChild(root, row)

proc quitWhenCaptureDone(doneFile: string) {.thread.} =
  for _ in 0 ..< WatcherMaxTicks:
    if fileExists(doneFile): break
    sleep(WatcherTickMs)
  gpui_quit()

suite "the window paints a two-stop conic gradient as a browser does":

  test "five discs at 0, 16, 50, 99 and 100 % match Chromium's":
    require getEnv("WAYLAND_DISPLAY").len > 0
    require findExe("grim").len > 0
    let fixture = currentSourcePath().parentDir / "fixtures" / "conic" /
                  "browser-strip.ppm"
    require fileExists(fixture)
    let browser = readPpm(fixture)
    require browser.w == Percents.len * DiscPx + (Percents.len - 1) * GapPx
    require browser.h == DiscPx

    gpui_reset_tree()
    gpui_reset_windows()
    let capture = currentSourcePath().parentDir.parentDir / "scripts" /
                  "wayland-capture-frame.sh"
    let capDir = getTempDir() / ("isonim-gpui-conic-" & $getCurrentProcessId())
    createDir(capDir)
    let ppm = capDir / "frame.ppm"
    let helper = startProcess(findExe("bash"),
      args = @[capture, ppm, $CaptureTimeoutSec], options = {poParentStreams})
    var watcher: Thread[string]
    createThread(watcher, quitWhenCaptureDone, ppm & ".done")
    gpui_quit_after_ms(CaptureBackstopMs)
    gpui_launch("IsoNim GPUI conic".cstring, 400.0, 200.0, buildScene)
    joinThread(watcher)
    discard waitForExit(helper)
    close(helper)
    if not fileExists(ppm):
      checkpoint("no frame captured; see " & ppm & ".log")
    require fileExists(ppm)
    let f = readPpm(ppm)
    require f.w >= browser.w and f.h >= browser.h

    let area = PI * float(DiscPx * DiscPx) / 4.0
    for k, p in Percents:
      var ours, theirs = 0
      for y in 0 ..< DiscPx:
        for x in discLeft(k) ..< discLeft(k) + DiscPx:
          if isFirst(f.rgbAt(x, y)): inc ours
          if isFirst(browser.rgbAt(x, y)): inc theirs
      echo &"    {p:>3} %: red-dominant pixels browser {theirs}, window {ours}"
      check float(abs(ours - theirs)) <= CountTolerance * area

    var pure, mismatched = 0
    for y in 1 ..< browser.h - 1:
      for x in 1 ..< browser.w - 1:
        let c = browser.rgbAt(x, y)
        if c notin [(255, 0, 0), (0, 0, 255), (255, 255, 255)]: continue
        var uniform = true
        for dy in -1 .. 1:
          for dx in -1 .. 1:
            if browser.rgbAt(x + dx, y + dy) != c: uniform = false
        if not uniform: continue
        if nearStopLine(x, y): continue
        inc pure
        let o = f.rgbAt(x, y)
        if abs(o[0] - c[0]) > PureTolerance or abs(o[1] - c[1]) > PureTolerance or
            abs(o[2] - c[2]) > PureTolerance:
          inc mismatched
          if mismatched <= 10:
            echo &"    ({x},{y}): browser {c}, window {o}"
    echo &"    pure pixels compared {pure}, differing {mismatched}"
    check pure > 0
    check mismatched == 0
