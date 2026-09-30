## test_gui_borders.nim — the window draws the BORDERS, the WEIGHT and the
## ABSOLUTE PLACEMENT the render plan carries, and a real pointer reaches an
## element with its position.
##
## `apply_styles_to_div` used to read `border-*` and `font-weight` into the
## plan and never draw them: a consumer outlining its focused pane or bolding
## its active tab saw the plan say so and the window show neither. This suite
## reads a REAL WINDOW's pixels on a headless sway (`wayland-capture-frame.sh`
## and `grim`, as `test_gui.nim`'s pixel case does):
##
##   * a box with a 3-pixel `#ff0000` border is framed on all four sides by
##     exactly a 3-pixel red ring, and nothing else on screen is red;
##   * a per-side border (`border-left-width` only) is red on that side only;
##   * a bold label's glyph STEMS are wider than the same label's at normal
##     weight (the dark run across the middle of each `I`);
##   * an absolutely positioned translucent quad (`#rrggbbaa`,
##     `position: absolute; top; left`) is drawn over the box it overlaps,
##     BLENDED with it rather than covering it;
##   * the compositor's own pointer — a virtual pointer DEVICE
##     (`tools/virtual_pointer.c`), so the seat has a real `wl_pointer` —
##     delivers `mousedown`, `mousemove` and `mouseup` to the box's listeners
##     with the window position (`pointerOf`), and a wheel turned two
##     notches arrives as a `wheel` event with its delta;
##   * a row's `padding-left` moves its first child in by exactly that much,
##     and a `flex-grow` child fills the row to its right edge;
##   * the host's TICK (`gpui_set_tick`) runs on the loop's own thread, at its
##     interval, and stops the moment the host disarms it from inside the
##     callback (it ticks `TickLimit` times and never again, although the
##     window stays open for seconds after).
##
## Run by `just test-gui-borders` (headless sway, the windowed shim). No
## mocks: the real shim, a real window, a real compositor.

import unittest
import std/[os, osproc, streams, strformat, strutils, times]

import isonim_gpui/renderer
import isonim_gpui/bindings

const
  RootBg = "#ffffff"
  BoxBg = "#202020"
  BorderColor = "#ff0000"
  BorderPx = 3
  BoxW = 200
  BoxH = 120
  SideBoxW = 120
  SideBoxH = 80
  LabelText = "IIIIIIII"
  LabelPx = 48
  QuadColor = "#00ff0080"     ## green at half alpha
  QuadTop = 30
  QuadLeft = 40
  QuadW = 60
  QuadH = 40
  RowW = 300
  RowPadPx = 20
  RowFixedW = 30

  TickMs = 100'u32
  TickLimit = 5
    ## The tick disarms itself on its fifth call.

  CaptureTimeoutSec = 30
  CaptureBackstopMs = 60_000'u32
  WatcherTickMs = 50
  WatcherMaxTicks = 1200

type
  Frame = object
    w, h: int
    pixels: string

var
  tickCount = 0
  tickOffThread = 0
    ## Ticks that ran on a thread other than the one that launched the loop.
  launchThread = 0
  pointerLog: seq[(GpuiEventKind, float, float)]
    ## What the box's listeners were handed, in order.
  wheelDy: seq[float]
    ## Each wheel event's vertical delta, in pixels.

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

proc isRed(c: (int, int, int)): bool = c[0] > 200 and c[1] < 60 and c[2] < 60
proc isDark(c: (int, int, int)): bool = c[0] + c[1] + c[2] < 3 * 110

proc buildScene(root: GpuiElement) {.cdecl.} =
  ## A column: the bordered box (with the translucent quad over it and the
  ## pointer listeners on it), the one-sided box, the normal label and the
  ## bold label, each at a known place.
  let r = GpuiRenderer()
  r.setStyle(root, "background-color", RootBg)
  r.setStyle(root, "width", "100%")
  r.setStyle(root, "height", "100%")
  r.setStyle(root, "flex-direction", "column")
  r.setStyle(root, "gap", "20px")

  let box = r.createElement("div")
  r.setStyle(box, "background-color", BoxBg)
  r.setStyle(box, "width", $BoxW & "px")
  r.setStyle(box, "height", $BoxH & "px")
  r.setStyle(box, "border-width", $BorderPx & "px")
  r.setStyle(box, "border-color", BorderColor)
  for name in ["mousedown", "mousemove", "mouseup", "wheel"]:
    r.addEventListener(box, name, proc(ev: GpuiEvent) =
      let p = pointerOf(ev)
      if p.valid:
        pointerLog.add (ev.kind, p.x, p.y)
        if ev.kind == gekWheel:
          wheelDy.add p.dy)
  let quad = r.createElement("div")
  r.setStyle(quad, "position", "absolute")
  r.setStyle(quad, "top", $QuadTop & "px")
  r.setStyle(quad, "left", $QuadLeft & "px")
  r.setStyle(quad, "width", $QuadW & "px")
  r.setStyle(quad, "height", $QuadH & "px")
  r.setStyle(quad, "background-color", QuadColor)
  r.appendChild(box, quad)
  r.appendChild(root, box)

  let side = r.createElement("div")
  r.setStyle(side, "background-color", BoxBg)
  r.setStyle(side, "width", $SideBoxW & "px")
  r.setStyle(side, "height", $SideBoxH & "px")
  r.setStyle(side, "border-left-width", $BorderPx & "px")
  r.setStyle(side, "border-color", BorderColor)
  r.appendChild(root, side)

  # A ROW with a left padding, a fixed child and a GROWING child: the
  # padding moves the first child in, and the grower fills the row to its
  # right edge (`padding-left`, `flex-grow`).
  let row = r.createElement("div")
  r.setStyle(row, "flex-direction", "row")
  r.setStyle(row, "width", $RowW & "px")
  r.setStyle(row, "height", "20px")
  r.setStyle(row, "background-color", RootBg)
  r.setStyle(row, "padding-left", $RowPadPx & "px")
  let fixed = r.createElement("div")
  r.setStyle(fixed, "width", $RowFixedW & "px")
  r.setStyle(fixed, "height", "20px")
  r.setStyle(fixed, "background-color", "#0000ff")
  let grow = r.createElement("div")
  r.setStyle(grow, "height", "20px")
  r.setStyle(grow, "flex-grow", "1")
  r.setStyle(grow, "background-color", "#00ff00")
  r.appendChild(row, fixed)
  r.appendChild(row, grow)
  r.appendChild(root, row)

  for weight in ["normal", "bold"]:
    let label = r.createElement("div")
    r.setStyle(label, "color", "#000000")
    r.setStyle(label, "font-size", $LabelPx & "px")
    r.setStyle(label, "font-weight", weight)
    r.setStyle(label, "font-family", "DejaVu Sans")
    r.setAttribute(label, "data-weight", weight)
    r.appendChild(label, r.createTextNode(LabelText))
    r.appendChild(root, label)

proc onTick() {.cdecl.} =
  inc tickCount
  if getThreadId() != launchThread:
    inc tickOffThread
  if tickCount == TickLimit:
    gpui_set_tick(0, nil)

proc quitWhenCaptureDone(doneFile: string) {.thread.} =
  ## Quit once BOTH the capture has its frame and the pointer script has
  ## finished (`<frame>.pointer-done`): a window that quit mid-gesture would
  ## make the pointer's missing events look like a wiring defect.
  for _ in 0 ..< WatcherMaxTicks:
    if fileExists(doneFile) and fileExists(doneFile & ".pointer-done"):
      break
    sleep(WatcherTickMs)
  gpui_quit()

proc drivePointer(doneFile: string) {.thread.} =
  ## THE COMPOSITOR'S OWN POINTER: `build/virtual-pointer` (built by
  ## `scripts/build-virtual-pointer.sh`) gives the headless sway a pointer
  ## device and moves, presses, drags and releases it — events routed by the
  ## compositor to the surface under the cursor, as a mouse's would be. It
  ## waits for the window to be mapped first.
  let tool = currentSourcePath().parentDir.parentDir / "build" /
             "virtual-pointer"
  let p = startProcess(tool, args = @["1920", "1080"], options = {})
  let input = p.inputStream
  for cmd in ["sleep 4000", "abs 50 50", "sleep 300", "down", "sleep 300",
              "abs 70 60", "sleep 300", "abs 90 70", "sleep 300", "up",
              "sleep 300", "wheel 2", "sleep 500", "quit"]:
    input.writeLine(cmd)
    input.flush()
  discard p.waitForExit()
  stderr.writeLine("    virtual pointer: " & p.errorStream.readAll().strip())
  p.close()
  writeFile(doneFile & ".pointer-done", "")

suite "the window draws what the plan carries: borders, weight, placement, pointer":

  test "a bordered box, a bold label, a translucent quad and a real pointer":
    require getEnv("WAYLAND_DISPLAY").len > 0
    require findExe("grim").len > 0
    require fileExists(currentSourcePath().parentDir.parentDir / "build" /
                       "virtual-pointer")
    gpui_reset_tree()
    gpui_reset_windows()
    resetCallbacks()
    let capture = currentSourcePath().parentDir.parentDir / "scripts" /
                  "wayland-capture-frame.sh"
    let capDir = getTempDir() / ("isonim-gpui-borders-" & $getCurrentProcessId())
    createDir(capDir)
    let ppm = capDir / "frame.ppm"
    let helper = startProcess(findExe("bash"),
      args = @[capture, ppm, $CaptureTimeoutSec], options = {poParentStreams})
    var watcher: Thread[string]
    createThread(watcher, quitWhenCaptureDone, ppm & ".done")
    var typist: Thread[string]
    createThread(typist, drivePointer, ppm & ".done")
    gpui_quit_after_ms(CaptureBackstopMs)
    launchThread = getThreadId()
    gpui_set_tick(TickMs, onTick)
    let launchedAt = epochTime()
    gpui_launch("IsoNim GPUI borders".cstring, 800.0, 600.0, buildScene)
    let openMs = int((epochTime() - launchedAt) * 1000)
    joinThread(watcher)
    joinThread(typist)
    discard waitForExit(helper)
    close(helper)
    if not fileExists(ppm):
      checkpoint("no frame captured; see " & ppm & ".log")
    require fileExists(ppm)
    let f = readPpm(ppm)

    # ---- the ring: exactly BorderPx red on every side of the box ------------
    var red = 0
    var x0 = int.high
    var y0 = int.high
    var x1 = -1
    var y1 = -1
    for y in 0 ..< f.h:
      for x in 0 ..< f.w:
        if isRed(f.rgbAt(x, y)):
          inc red
          x0 = min(x0, x); x1 = max(x1, x)
          y0 = min(y0, y); y1 = max(y1, y)
    let boxRing = BoxW * BoxH - (BoxW - 2 * BorderPx) * (BoxH - 2 * BorderPx)
    let sideTop = BoxH + 20
    let sideStrip = BorderPx * SideBoxH
    echo &"    red pixels {red} (ring {boxRing} + side strip {sideStrip}); " &
         &"bbox ({x0},{y0})-({x1},{y1})"
    check red == boxRing + sideStrip
    check x0 == 0 and y0 == 0 and x1 == BoxW - 1
    # The ring is closed: each side's middle pixel is red, the pixel just
    # inside it is not.
    check isRed(f.rgbAt(BoxW div 2, 0)) and isRed(f.rgbAt(BoxW div 2, BoxH - 1))
    check isRed(f.rgbAt(0, BoxH div 2)) and isRed(f.rgbAt(BoxW - 1, BoxH div 2))
    check not isRed(f.rgbAt(BoxW div 2, BorderPx))
    check not isRed(f.rgbAt(BorderPx, BoxH div 2))
    # The one-sided box: red on its left, not on its right, top or bottom.
    check isRed(f.rgbAt(0, sideTop + SideBoxH div 2))
    check not isRed(f.rgbAt(SideBoxW - 1, sideTop + SideBoxH div 2))
    check not isRed(f.rgbAt(SideBoxW div 2, sideTop))

    # ---- the quad: blended over the box, not covering it --------------------
    let q = f.rgbAt(QuadLeft + QuadW div 2, QuadTop + QuadH div 2)
    let beside = f.rgbAt(QuadLeft + QuadW + 10, QuadTop + QuadH div 2)
    echo &"    quad {q}, box beside it {beside}"
    check q[1] > beside[1] + 60          # greener than the box
    check q[1] < 250 and q[0] < 60       # but not opaque green: blended
    check beside == (0x20, 0x20, 0x20)

    # ---- weight: the bold label's stems are wider ---------------------------
    # Below the two boxes the labels are the only dark ink: every row whose
    # dark runs number exactly the label's letters crosses one of the two
    # labels. The first band of such rows is the normal label, the second
    # the bold one, and each band's widest run is its stem width.
    proc runsOf(y: int): seq[int] =
      var run = 0
      for x in 0 ..< min(f.w, 700):
        if isDark(f.rgbAt(x, y)): inc run
        elif run > 0:
          result.add run
          run = 0
    var bands: seq[seq[int]] = @[]
    var inBand = false
    for y in (sideTop + SideBoxH + 1) ..< f.h:
      let runs = runsOf(y)
      if runs.len == LabelText.len:
        if not inBand:
          bands.add @[]
          inBand = true
        bands[^1].add max(runs)
      else:
        inBand = false
    echo &"    label bands: {bands.len}; stem widths per band: {bands}"
    check bands.len == 2
    if bands.len == 2:
      check max(bands[1]) > max(bands[0])

    # ---- the pointer: down, move, up, each with its window position ---------
    echo &"    pointer events {pointerLog}"
    var kinds: seq[GpuiEventKind] = @[]
    for (k, _, _) in pointerLog:
      if kinds.len == 0 or kinds[^1] != k: kinds.add k
    check gekPointerDown in kinds and gekPointerUp in kinds and
          gekPointerMove in kinds
    for (k, x, y) in pointerLog:
      if k == gekPointerDown:
        check abs(x - 50.0) < 1.5 and abs(y - 50.0) < 1.5
      if k == gekPointerUp:
        check abs(x - 90.0) < 1.5 and abs(y - 70.0) < 1.5
    # ---- the wheel: two notches down, delivered with a delta ---------------
    echo &"    wheel deltas {wheelDy}"
    check wheelDy.len >= 1
    var total = 0.0
    for d in wheelDy: total += d
    check total != 0.0

    # ---- padding-left and flex-grow ------------------------------------------
    var rowY = -1
    for y in 0 ..< f.h:
      if f.rgbAt(RowPadPx + 2, y) == (0, 0, 255):
        rowY = y
        break
    echo &"    the padded row at y={rowY}"
    check rowY >= 0
    if rowY >= 0:
      check f.rgbAt(RowPadPx - 1, rowY) == (255, 255, 255)   # the padding
      check f.rgbAt(RowPadPx, rowY) == (0, 0, 255)
      check f.rgbAt(RowPadPx + RowFixedW - 1, rowY) == (0, 0, 255)
      check f.rgbAt(RowPadPx + RowFixedW, rowY) == (0, 255, 0)
      check f.rgbAt(RowW - 1, rowY) == (0, 255, 0)          # grown to the edge
      check f.rgbAt(RowW, rowY) == (255, 255, 255)

    # ---- the tick: on the loop's thread, at its interval, disarmable -------
    echo &"    ticks {tickCount} (limit {TickLimit}) over {openMs} ms open; " &
         &"off-thread {tickOffThread}"
    check openMs > int(TickMs) * (TickLimit + 5)   # time for many more
    check tickCount == TickLimit
    check tickOffThread == 0
    removeDir(capDir)
