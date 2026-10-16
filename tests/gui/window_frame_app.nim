## The application the window-frame suite (`tests/test_gui_window_frame.nim`)
## drives: one window whose content is a known colour, reporting what the
## shim tells it — resizes, window-state changes and pointer presses — as
## JSON lines appended to `$GWIN_REPORT`.
##
## It does nothing else on purpose. Everything the suite asserts about the
## frame is read from the COMPOSITOR (window position and size) and from the
## screen (pixels); this report is the application's side of the comparison
## — the content size it was told and the coordinates a press reached it at.
##
## Environment:
##   GWIN_REPORT    the file to append to (required)
##   GWIN_QUIT_MS   quit after this many milliseconds (default 120000)
##   GWIN_BORDER    the frame's border colour (`data-isonim-window-border`),
##                  so a suite can find the border on a black desktop
##   GWIN_TITLEBAR_BG  the renderer's fallback title bar colour
##                  (`data-isonim-titlebar-background`)
##   GWIN_REGIONS   `1`: also draw a title bar out of window regions — see
##                  `buildTitleBar` — and act on keys: m minimize, x toggle
##                  maximize, f toggle fullscreen, q close, v refuse the next
##                  close (`onClose` returns false once)

import std/[json, os, strutils]
import isonim_gpui/[bindings, renderer, window, window_environment]

const
  AppId* = "isonim-gpui-frame-test"
  ContentColor* = "#336699"

let reportPath = getEnv("GWIN_REPORT")

proc emit(n: JsonNode) =
  if reportPath.len == 0: return
  let f = open(reportPath, fmAppend)
  f.writeLine($n)
  f.close()

proc stateOf(win: GpuiWindow): JsonNode =
  let env = windowEnvironment(win)
  var tiling = 0
  for e in win.tiledEdges: tiling = tiling or (1 shl ord(e))
  %*{"event": "state", "csd": win.clientDecorated, "tiling": tiling,
     "maximized": win.isMaximized, "active": win.isActive,
     "layout": win.buttonLayout, "right": $env.preferredRight,
     "left": $env.preferredLeft, "source": $env.placementSource}

const
  TitleBarH* = 40          ## the drag strip's height
  NoDragX* = 100           ## a `no-drag` child of the strip
  ListenerX* = 220         ## a sibling over the strip with a press listener
  PartW* = 80
  PartH* = 30
  ButtonSize* = 30         ## window buttons, from the right edge inwards
  ButtonGap* = 6
  ButtonMargin* = 10

var
  buttons: seq[GpuiElement]
  titleBar: GpuiElement
  refuseNextClose = false

proc box(r: GpuiRenderer; parent: GpuiElement; color: string;
         left, top, w, h: int; fromRight = false): GpuiElement =
  result = r.createElement("div")
  r.setStyle(result, "position", "absolute")
  r.setStyle(result, if fromRight: "right" else: "left", $left & "px")
  r.setStyle(result, "top", $top & "px")
  r.setStyle(result, "width", $w & "px")
  r.setStyle(result, "height", $h & "px")
  r.setStyle(result, "background-color", color)
  r.appendChild(parent, result)

proc layoutButtons(r: GpuiRenderer; win: GpuiWindow) =
  ## The window buttons this environment supports, in the order it prefers,
  ## at the strip's right end (the suite reads the order back from the
  ## report). Redrawn whenever the window state changes.
  for b in buttons: r.removeChild(titleBar, b)
  buttons.setLen 0
  let env = windowEnvironment(win)
  let order = env.preferredLeft & env.preferredRight
  for i, b in order:
    let fromRight = ButtonMargin + (order.len - 1 - i) * (ButtonSize + ButtonGap)
    let el = r.box(titleBar, "#ddddcc", fromRight, (TitleBarH - ButtonSize) div 2,
                   ButtonSize, ButtonSize, fromRight = true)
    r.setAttribute(el, windowRegionAttribute, $b)
    buttons.add el
  emit(%*{"event": "buttons", "order": $order})

proc buildTitleBar(r: GpuiRenderer; root: GpuiElement; win: GpuiWindow) =
  ## A strip across the top that is a drag region, with a `no-drag` child and
  ## a sibling drawn over it that listens for presses (and is therefore
  ## implicitly no-drag), and the window buttons at its right end.
  titleBar = r.createElement("div")
  r.setStyle(titleBar, "position", "absolute")
  r.setStyle(titleBar, "left", "0px")
  r.setStyle(titleBar, "top", "0px")
  r.setStyle(titleBar, "width", "100%")
  r.setStyle(titleBar, "height", $TitleBarH & "px")
  r.setStyle(titleBar, "background-color", "#224466")
  r.setAttribute(titleBar, windowRegionAttribute, $wrDrag)
  r.appendChild(root, titleBar)
  let noDrag = r.box(titleBar, "#446688", NoDragX, 5, PartW, PartH)
  r.setAttribute(noDrag, windowRegionAttribute, $wrNoDrag)
  let listener = r.box(root, "#668844", ListenerX, 5, PartW, PartH)
  r.addEventListener(listener, "mousedown", proc(ev: GpuiEvent) =
    emit(%*{"event": "listener-press"}))
  r.layoutButtons(win)
  win.onClose(proc(): bool =
    if refuseNextClose:
      refuseNextClose = false
      emit(%*{"event": "close-refused"})
      return false
    emit(%*{"event": "close-allowed"})
    true)
  # Keys: the imperative operations. The root takes keyboard focus.
  setFocusable(root)
  discard focusElement(root)
  r.addEventListener(root, "keydown", proc(ev: GpuiEvent) =
    emit(%*{"event": "key", "key": root.lastEventKey()})
    case root.lastEventKey()
    of "m": win.minimize()
    of "x": win.toggleMaximize()
    of "f": win.toggleFullscreen()
    of "q": win.requestClose()
    of "v": refuseNextClose = true
    else: discard)

proc build(root: GpuiElement) {.cdecl.} =
  let r = GpuiRenderer()
  r.setStyle(root, "background-color", ContentColor)
  r.setStyle(root, "width", "100%")
  r.setStyle(root, "height", "100%")
  let border = getEnv("GWIN_BORDER")
  if border.len > 0: r.setAttribute(root, "data-isonim-window-border", border)
  let barColor = getEnv("GWIN_TITLEBAR_BG")
  if barColor.len > 0: r.setAttribute(root, "data-isonim-titlebar-background", barColor)
  r.addEventListener(root, "mousedown", proc(ev: GpuiEvent) =
    let p = pointerOf(ev)
    if p.valid: emit(%*{"event": "press", "x": p.x, "y": p.y}))
  let win = launchWindow()
  win.onResize(proc(w, h: float) = emit(%*{"event": "resize", "w": w, "h": h}))
  # The size it opens at: a window the compositor never resizes (server-side
  # decorations, launch size kept) is never told one otherwise.
  emit(%*{"event": "resize", "w": win.width, "h": win.height})
  let regions = getEnv("GWIN_REGIONS") == "1"
  win.onWindowState(proc() =
    emit(stateOf(win))
    if regions: r.layoutButtons(win))
  if regions: r.buildTitleBar(root, win)

when isMainModule:
  if reportPath.len == 0:
    quit("window_frame_app: GWIN_REPORT is not set", 2)
  setAppId(AppId)
  setMinWindowSize(200, 150)
  gpui_quit_after_ms(uint32(parseInt(getEnv("GWIN_QUIT_MS", "120000"))))
  gpui_launch("GWIN frame test".cstring, 640, 400, build)
