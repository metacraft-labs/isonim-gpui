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

proc build(root: GpuiElement) {.cdecl.} =
  let r = GpuiRenderer()
  r.setStyle(root, "background-color", ContentColor)
  r.setStyle(root, "width", "100%")
  r.setStyle(root, "height", "100%")
  let border = getEnv("GWIN_BORDER")
  if border.len > 0: r.setAttribute(root, "data-isonim-window-border", border)
  r.addEventListener(root, "mousedown", proc(ev: GpuiEvent) =
    let p = pointerOf(ev)
    if p.valid: emit(%*{"event": "press", "x": p.x, "y": p.y}))
  let win = launchWindow()
  win.onResize(proc(w, h: float) = emit(%*{"event": "resize", "w": w, "h": h}))
  # The size it opens at: a window the compositor never resizes (server-side
  # decorations, launch size kept) is never told one otherwise.
  emit(%*{"event": "resize", "w": win.width, "h": win.height})
  win.onWindowState(proc() = emit(stateOf(win)))

when isMainModule:
  if reportPath.len == 0:
    quit("window_frame_app: GWIN_REPORT is not set", 2)
  setAppId(AppId)
  setMinWindowSize(200, 150)
  gpui_quit_after_ms(uint32(parseInt(getEnv("GWIN_QUIT_MS", "120000"))))
  gpui_launch("GWIN frame test".cstring, 640, 400, build)
