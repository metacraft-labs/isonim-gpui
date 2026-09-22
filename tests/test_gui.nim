## GUI tests for isonim-gpui.
##
## Two builds, and they are not the same subject:
##
##   nim c -r tests/test_gui.nim
##     The shadow tree and the render PLAN. No shim feature flags, no
##     display server, no window. This is what `ci/run-suite.sh` runs.
##
##   just test-gui   (== `-d:gpuiBackend` against a shim built with
##                    `--features gpui-backend`, under headless sway)
##     The same cases, plus the windowed ones: `gpui_launch` really does
##     enter GPUI's platform event loop and really does open a window,
##     and the last suite in this file reads the resulting PIXELS back
##     off the compositor.
##
## RS-M14b — WHY THAT LAST SUITE EXISTS. Every assertion in this file
## used to be over the render plan or the in-process window state
## machine, and the comment above the launch suite said in as many words
## that the launch cases ran *without* the backend. So "the GUI tests"
## tested the plan and never the rendering — which is how a Cargo feature
## that resolved `gpui_linux` to ZERO features, and therefore disabled
## all Linux windowing, survived in the tree until CI happened to run it.
## A plan assertion cannot fail on a renderer that does not exist. A
## pixel assertion can, and now does.
##
## Build & run:
##   LD_LIBRARY_PATH=rust/target/debug nim c -r --path:../isonim/src tests/test_gui.nim

import unittest
import std/[json, strutils]
import isonim_gpui/renderer
import isonim_gpui/bindings

when defined(gpuiBackend):
  import std/[os, osproc, strformat, times]

# ============================================================================
# Helpers
# ============================================================================

# Trap 13 (Verification-Harness-Traps.md): written as a `proc`, the
# `check` below sets `unittest`'s MODULE-LEVEL `testStatusIMPL` instead
# of the running test's local, so every case printed `Check failed:` and
# then reported `[OK]`. Measured on 2026-09-15 by planting
# `check jsonStr.len < 0` into the HEAD (`proc`) form of this helper and
# running it: **seven** `Check failed:` lines — one per `getPlan` CALL
# SITE in this file — fourteen `[OK]`s, and not one `[FAILED]`. (An
# earlier draft of this comment said eight. Eight is what `grep -c
# getPlan` answers, because it counts the declaration too. The run says
# seven, and the run is the measurement.)
#
# The process still exited 1, because `programResult` reads the same
# module-level status the `check` corrupted. So the defect is precisely
# that every CASE reports `[OK]`, and the only instrument that noticed
# was the exit code — which is why `ci/run-suite.sh` takes every verdict
# from an rc and never from an `[OK]` count.
#
# As a `template` this expands inside the test body, where
# `testStatusIMPL` is the test's own.
template getPlan(r: GpuiRenderer; node: GpuiElement): JsonNode =
  ## Build the render plan for a node and parse it as JSON.
  let jsonStr = r.renderPlanJson(node)
  check jsonStr.len > 0
  parseJson(jsonStr)

# ============================================================================
# Render Plan Smoke Tests (no display server required)
# ============================================================================

suite "GUI - Render Plan Smoke Tests":
  setup:
    gpui_reset_tree()
    resetCallbacks()

  test "create_element_and_verify_render_plan":
    let r = GpuiRenderer()
    let root = r.createElement("div")
    let child = r.createElement("span")
    r.appendChild(root, child)
    r.setTextContent(child, "Hello GPUI")

    check r.verifyRenderPlan(root)
    check r.renderPlanElementCount(root) > 0

    let plan = r.getPlan(root)
    check plan["kind"].getStr() == "Div"  # div maps to Div

  test "styled_element_in_render_plan":
    let r = GpuiRenderer()
    let root = r.createElement("div")
    r.setStyle(root, "background-color", "#ff0000")
    r.setStyle(root, "width", "200px")
    r.setStyle(root, "height", "100px")

    let plan = r.getPlan(root)
    check plan.hasKey("styles")
    let styles = plan["styles"]
    check styles["bg"].getStr() == "#ff0000"
    check styles["w"].getStr() == "200px"
    check styles["h"].getStr() == "100px"

  test "event_handler_in_render_plan":
    let r = GpuiRenderer()
    let btn = r.createElement("button")
    var clicked = false
    r.addEventListener(btn, "click", proc() = clicked = true)

    let plan = r.getPlan(btn)
    check plan["has_click_handler"].getBool() == true

    # Verify the event actually fires
    fireEvent(btn, "click")
    check clicked == true

  test "nested_tree_render_plan":
    let r = GpuiRenderer()
    let root = r.createElement("div")
    let header = r.createElement("h1")
    let nav = r.createElement("nav")
    let item1 = r.createElement("span")
    let item2 = r.createElement("span")

    r.appendChild(root, header)
    r.appendChild(root, nav)
    r.appendChild(nav, item1)
    r.appendChild(nav, item2)
    r.setTextContent(header, "Title")
    r.setTextContent(item1, "Item 1")
    r.setTextContent(item2, "Item 2")

    check r.verifyRenderPlan(root)
    check r.renderPlanElementCount(root) >= 5

    let plan = r.getPlan(root)
    check plan["children"].len == 2

  test "counter_app_render_plan":
    # Build a simple counter app and verify its render plan
    let r = GpuiRenderer()
    let root = r.createElement("div")
    let countLabel = r.createElement("span")
    let incBtn = r.createElement("button")
    let decBtn = r.createElement("button")

    r.appendChild(root, countLabel)
    r.appendChild(root, incBtn)
    r.appendChild(root, decBtn)

    r.setTextContent(countLabel, "0")
    r.setTextContent(incBtn, "+")
    r.setTextContent(decBtn, "-")

    var count = 0
    r.addEventListener(incBtn, "click", proc() =
      count += 1
      r.setTextContent(countLabel, $count)
    )
    r.addEventListener(decBtn, "click", proc() =
      count -= 1
      r.setTextContent(countLabel, $count)
    )

    check r.verifyRenderPlan(root)
    check textContent(countLabel) == "0"

    fireEvent(incBtn, "click")
    check textContent(countLabel) == "1"
    check count == 1

    fireEvent(incBtn, "click")
    fireEvent(incBtn, "click")
    check textContent(countLabel) == "3"

    fireEvent(decBtn, "click")
    check textContent(countLabel) == "2"

    # Verify render plan still valid after mutations
    check r.verifyRenderPlan(root)

  test "nil_node_returns_empty_plan":
    let r = GpuiRenderer()
    let json = r.renderPlanJson(nil)
    check json == ""
    check not r.verifyRenderPlan(nil)
    check r.renderPlanElementCount(nil) == 0

  test "text_node_in_render_plan":
    let r = GpuiRenderer()
    let text = r.createTextNode("hello GUI")
    let plan = r.getPlan(text)
    check plan["kind"].getStr == "TextNode"
    check plan["text"].getStr == "hello GUI"

  test "multiple_styles_propagate":
    let r = GpuiRenderer()
    let el = r.createElement("div")
    r.setStyle(el, "background-color", "blue")
    r.setStyle(el, "padding", "10")
    r.setStyle(el, "gap", "5")
    let plan = r.getPlan(el)
    check plan["styles"]["bg"].getStr == "blue"
    check plan["styles"]["p"].getStr == "10"
    check plan["styles"]["gap"].getStr == "5"

  test "no_handlers_by_default":
    let r = GpuiRenderer()
    let el = r.createElement("div")
    let plan = r.getPlan(el)
    check plan["has_click_handler"].getBool == false
    check plan["has_input_handler"].getBool == false
    check plan["event_names"].len == 0

# ============================================================================
# Launch Integration Tests
# ============================================================================
#
# These exercise the full Nim -> FFI -> Rust -> callback -> tree pipeline
# through `gpui_launch`, and what `gpui_launch` DOES depends on how the
# shim was built:
#
#   default build   the root_builder callback runs, the shadow tree is
#                   built, and the function returns. No window.
#
#   gpui-backend    the same, and then `Application::run(...)` opens a
#                   real window and blocks until the event loop stops.
#
# The second case used to be unreachable, because the Cargo feature
# enabled no Linux windowing backend at all. When that was fixed, these
# five cases stopped terminating — measured 2026-09-17: the nine
# render-plan cases above printed `[OK]`, this suite printed its header,
# and the process had to be killed (rc=124 under a 90s cap). The
# `unreachable!()` had been masking a hang.
#
# `gpui_quit_after_ms` (RS-M14b) is the fix: it bounds the event loop
# from OUTSIDE it, which is a thing GPUI itself offers no way to do. The
# assertions below are all about the shadow tree the builder produced, so
# they hold either way; what changes under `-d:gpuiBackend` is that a
# window is opened and closed five times in a row, which is coverage the
# windowed pixel suite at the bottom of this file does not give (it
# launches once).
const LaunchAutoQuitMs = 750'u32
  ## Long enough for the window to open on the software-rendering path
  ## measured here (~700 ms to first frame under headless sway with
  ## `WLR_RENDERER=pixman`), short enough that five of them are a few
  ## seconds. Unused in the default build.

suite "GUI - Launch Integration Tests":
  setup:
    gpui_reset_tree()
    gpui_reset_windows()
    resetCallbacks()
    when defined(gpuiBackend):
      # Must come AFTER `gpui_reset_windows`, which clears it.
      gpui_quit_after_ms(LaunchAutoQuitMs)

  test "gpui_launch_calls_root_builder":
    var builderCalled = false
    var rootElement: GpuiElement = nil

    proc builder(root: GpuiElement) {.cdecl.} =
      builderCalled = true
      rootElement = root

    gpui_launch("Test App".cstring, 800.0, 600.0, builder)

    check builderCalled
    check rootElement != nil

  test "gpui_launch_root_builder_can_build_tree":
    var rootEl: GpuiElement = nil

    proc builder(root: GpuiElement) {.cdecl.} =
      rootEl = root
      # Build a UI tree inside the callback
      let r = GpuiRenderer()
      let header = r.createElement("h1")
      let btn = r.createElement("button")
      r.appendChild(root, header)
      r.appendChild(root, btn)
      r.setTextContent(header, "Hello from Nim!")
      r.setTextContent(btn, "Click me")

    gpui_launch("Builder Test".cstring, 640.0, 480.0, builder)

    # After launch returns, verify the tree was built
    check rootEl != nil
    check childCount(rootEl) == 2
    check textContent(rootEl).contains("Hello from Nim!")
    check textContent(rootEl).contains("Click me")

  test "gpui_launch_render_plan_valid_after_build":
    var rootEl: GpuiElement = nil

    proc builder(root: GpuiElement) {.cdecl.} =
      rootEl = root
      let r = GpuiRenderer()
      let container = r.createElement("div")
      let label = r.createElement("span")
      r.appendChild(root, container)
      r.appendChild(container, label)
      r.setTextContent(label, "Render plan test")
      r.setStyle(container, "background-color", "blue")
      r.setStyle(container, "width", "300px")

    gpui_launch("Plan Test".cstring, 800.0, 600.0, builder)

    let r = GpuiRenderer()
    check r.verifyRenderPlan(rootEl)
    check r.renderPlanElementCount(rootEl) >= 3

    let planJson = r.renderPlanJson(rootEl)
    check planJson.len > 0
    let plan = parseJson(planJson)
    check plan["kind"].getStr == "Div"  # root tag maps to Div
    check plan["children"].len >= 1

  test "gpui_launch_event_handlers_work":
    var clickCount = 0
    var btnEl: GpuiElement = nil

    proc builder(root: GpuiElement) {.cdecl.} =
      let r = GpuiRenderer()
      btnEl = r.createElement("button")
      r.appendChild(root, btnEl)
      r.setTextContent(btnEl, "0")
      r.addEventListener(btnEl, "click", proc() =
        clickCount += 1
        r.setTextContent(btnEl, $clickCount)
      )

    gpui_launch("Event Test".cstring, 400.0, 300.0, builder)

    check btnEl != nil
    check textContent(btnEl) == "0"

    fireEvent(btnEl, "click")
    check clickCount == 1
    check textContent(btnEl) == "1"

    fireEvent(btnEl, "click")
    fireEvent(btnEl, "click")
    check clickCount == 3
    check textContent(btnEl) == "3"

  test "gpui_launch_counter_app_e2e":
    var rootEl, countLabel, incBtn, decBtn: GpuiElement
    var count = 0

    proc builder(root: GpuiElement) {.cdecl.} =
      rootEl = root
      let r = GpuiRenderer()
      countLabel = r.createElement("span")
      incBtn = r.createElement("button")
      decBtn = r.createElement("button")
      r.appendChild(root, countLabel)
      r.appendChild(root, incBtn)
      r.appendChild(root, decBtn)
      r.setTextContent(countLabel, "Count: 0")
      r.setTextContent(incBtn, "+")
      r.setTextContent(decBtn, "-")
      r.addEventListener(incBtn, "click", proc() =
        count += 1
        r.setTextContent(countLabel, "Count: " & $count)
      )
      r.addEventListener(decBtn, "click", proc() =
        count -= 1
        r.setTextContent(countLabel, "Count: " & $count)
      )

    gpui_launch("Counter".cstring, 400.0, 300.0, builder)

    # Verify initial state
    check textContent(countLabel) == "Count: 0"
    let r = GpuiRenderer()
    check r.verifyRenderPlan(rootEl)

    # Simulate user interactions
    fireEvent(incBtn, "click")
    fireEvent(incBtn, "click")
    fireEvent(incBtn, "click")
    check textContent(countLabel) == "Count: 3"

    fireEvent(decBtn, "click")
    check textContent(countLabel) == "Count: 2"

    # Verify render plan still valid after mutations
    check r.verifyRenderPlan(rootEl)
    check r.renderPlanElementCount(rootEl) >= 4

# ============================================================================
# GPUI Window Tests (require a Wayland compositor)
# ============================================================================

when defined(gpuiBackend):
  suite "GUI - GPUI Backend Compile Check":
    test "gpui_backend_feature_enabled":
      # When compiled with -d:gpuiBackend, the Rust shim should be built
      # with --features gpui-backend, which enables the actual GPUI rendering.
      # This case is about the in-process window REGISTRY only; the event
      # loop and the pixels it produces are the subject of the suite below.
      gpui_reset_tree()
      gpui_reset_windows()

      let winId = gpui_create_window("Backend Test".cstring, 640.0, 480.0)
      check winId > 0
      check gpui_window_state(winId) == 1  # Created state

      check gpui_show_window(winId) == 1
      check gpui_window_state(winId) == 2  # Visible state

      check gpui_close_window(winId) == 1
      check gpui_window_state(winId) == 4  # Closed state

      gpui_destroy_window(winId)

  # ==========================================================================
  # Windowed pixel capture (RS-M14b) — THE FIRST ASSERTION IN THIS LANE
  # OVER A DRAWN PIXEL.
  # ==========================================================================
  #
  # Everything above reads the shadow tree or the render plan: data the
  # shim produced on its way to the renderer. This suite reads what the
  # COMPOSITOR ended up holding, which is the only evidence that the
  # renderer ran at all.
  #
  # HOW IT IS ARMED. The scene is three flat colours with exact sizes and
  # an exact gap, and every one of those numbers is asserted twice over:
  # as a pixel COUNT and as a bounding BOX. So
  #
  #   * a window that opens and paints nothing      -> no frame captured,
  #                                                    or a blank one: red
  #   * a renderer that drops a child               -> that colour's count
  #                                                    is 0: red
  #   * a layout that ignores `gap`                 -> yellow's x0 is 250,
  #                                                    not 300: red
  #   * a style key that stops being understood     -> the box is the
  #                                                    wrong size or the
  #                                                    wrong colour: red
  #   * anything else on screen at all              -> `other` is not 0:
  #                                                    red
  #
  # MEASURED, 2026-09-17, on headless sway with `WLR_RENDERER=pixman`.
  # The captured frame is byte-exact: the whole 1920x1080 output resolves
  # to exactly three colours, with no antialiasing and no sRGB drift, so
  # these are equalities rather than tolerances.
  #
  #   GREEN     magenta=2026100 cyan=37500 yellow=10000 other=0
  #             2026100 + 37500 + 10000 = 2073600 = 1920*1080.
  #             Five consecutive runs, all rc=0, settling at capture tick
  #             5 or 6 of a 120-tick budget.
  #
  #   RED (1)   the scene drawn with box A 260px wide instead of 250,
  #             assertions unchanged. FIVE checks moved:
  #               cyan.count 39000 (not 37500)
  #               cyan.x1    259   (not 249)
  #               yellow.x0  310   (not 300)
  #               yellow.x1  409   (not 399)
  #               magenta.count 2024600 (not 2026100)
  #             The yellow box moving with box A's width is the part that
  #             matters: the capture tracks the app's LAYOUT, not a
  #             stored picture.
  #
  #   RED (2)   the same test binary against a shim built WITHOUT
  #             `--features gpui-backend`, i.e. no windowing compiled in
  #             and no window ever opened — the exact shape of the
  #             feature-flag defect that started this. All FIFTEEN
  #             pre-existing cases still reported `[OK]`, including
  #             `gpui_launch_counter_app_e2e` and
  #             `gpui_backend_feature_enabled`. Only this case went red,
  #             on "no frame was captured", after 30s of a screen that
  #             stayed at 17 non-NUL bytes.
  #
  # Red (2) is the whole argument for this suite existing. The old
  # assertions cannot distinguish a renderer that works from a renderer
  # that is not there.
  #
  # WHY WAYLAND AND NOT XVFB — AND THIS COMMENT WAS WRONG UNTIL
  # 2026-09-22. It said Xvfb "has no DRI3, so wgpu gets no surface: the
  # window is `IsViewable` at its requested size and paints nothing".
  # Re-measured with the windowed shim: IT PAINTS. The `Xvfb -fbdir`
  # framebuffer goes from 303 non-NUL bytes of 8,297,632 with no client
  # to 5,184,303 with the window up, and the frame decoded out of it is
  # the whole front-end. The `libEGL DRI3` warning is still printed; wgpu
  # falls back to a software Vulkan device and renders. The reason this
  # suite is on Wayland is the CAPTURE: `grim` speaks
  # `zwlr_screencopy_manager_v1`, which does not exist on an X display,
  # and the pinned `ffmpeg` has no `x11grab`. Headless sway
  # (`WLR_BACKENDS=headless`) works, on gles2
  # and on the fully-software `WLR_RENDERER=pixman` path alike, and
  # exposes `zwlr_screencopy_manager_v1` so `grim` can read the output
  # back. Headless weston does NOT work: it advertises no `wl_seat` and
  # GPUI unwraps `None`. See `scripts/wayland-run-test.sh`.
  #
  # The requirements below are checked with `require`, not skipped. A
  # case that quietly does nothing when its environment is missing is the
  # same failure mode as a case that asserts nothing.

  const
    # The scene. Every number is asserted against the captured frame.
    SceneRootBg = "#ff00ff"   ## magenta
    SceneBoxABg = "#00ffff"   ## cyan
    SceneBoxBBg = "#ffff00"   ## yellow
    SceneBoxAW  = 250
    SceneBoxAH  = 150
    SceneBoxBW  = 100
    SceneBoxBH  = 100
    SceneGap    = 50

    ColMagenta = 0xff00ff'u32
    ColCyan    = 0x00ffff'u32
    ColYellow  = 0xffff00'u32

    CaptureTimeoutSec = 30
      ## How long `wayland-capture-frame.sh` waits for a settled frame.
    CaptureBackstopMs = 45_000'u32
      ## Hard bound on the event loop, armed via `gpui_quit_after_ms`.
      ## Strictly greater than the capture timeout so that, on a healthy
      ## run, it is the watcher's `gpui_quit()` that stops the loop and
      ## not this. The elapsed-time assertion below is what tells the two
      ## apart, so "the backstop saved us" cannot be read as a pass.
    WatcherTickMs = 50
    WatcherMaxTicks = 900   ## 45 s, matching the backstop.

  proc buildPixelScene(root: GpuiElement) {.cdecl.} =
    ## The root_builder handed to `gpui_launch`. Builds a magenta
    ## full-bleed row containing a cyan box and a yellow box separated by
    ## a fixed gap.
    let r = GpuiRenderer()
    r.setStyle(root, "background-color", SceneRootBg)
    r.setStyle(root, "width", "100%")
    r.setStyle(root, "height", "100%")
    r.setStyle(root, "flex-direction", "row")
    r.setStyle(root, "gap", $SceneGap & "px")

    let boxA = r.createElement("div")
    r.setStyle(boxA, "background-color", SceneBoxABg)
    r.setStyle(boxA, "width", $SceneBoxAW & "px")
    r.setStyle(boxA, "height", $SceneBoxAH & "px")
    r.appendChild(root, boxA)

    let boxB = r.createElement("div")
    r.setStyle(boxB, "background-color", SceneBoxBBg)
    r.setStyle(boxB, "width", $SceneBoxBW & "px")
    r.setStyle(boxB, "height", $SceneBoxBH & "px")
    r.appendChild(root, boxB)

  # --- Frame decoding -------------------------------------------------

  type
    CapturedFrame = object
      w, h: int
      pixels: string    ## w * h * 3 bytes, RGB

    ColourStat = object
      count: int
      x0, x1, y0, y1: int

    SceneStats = object
      magenta, cyan, yellow: ColourStat
      other: int
      otherSample: uint32
      otherX, otherY: int

  proc emptyStat(): ColourStat =
    ColourStat(count: 0, x0: int.high, x1: int.low, y0: int.high, y1: int.low)

  proc observe(s: var ColourStat; x, y: int) =
    inc s.count
    s.x0 = min(s.x0, x)
    s.x1 = max(s.x1, x)
    s.y0 = min(s.y0, y)
    s.y1 = max(s.y1, y)

  proc readPpm(path: string): CapturedFrame =
    ## Decode the binary PPM (P6) `grim -t ppm` writes.
    ##
    ## `doAssert`, not `check`: a `check` inside a plain `proc` writes the
    ## MODULE-level `testStatusIMPL` rather than the running case's, which
    ## is trap 13 and is what the `getPlan` template at the top of this
    ## file exists to avoid. `tools/trap13-assertion-helper-sweep.py`
    ## enforces it.
    let raw = readFile(path)
    var fields: seq[string] = @[]
    var i = 0
    while fields.len < 4 and i < raw.len:
      while i < raw.len and raw[i] in Whitespace:
        inc i
      if i < raw.len and raw[i] == '#':
        while i < raw.len and raw[i] != '\n':
          inc i
        continue
      var j = i
      while j < raw.len and raw[j] notin Whitespace:
        inc j
      if j > i:
        fields.add raw[i ..< j]
      i = j
    doAssert fields.len == 4, "not a PPM header: " & path
    doAssert fields[0] == "P6", "expected binary PPM (P6), got " & fields[0]
    doAssert fields[3] == "255", "expected an 8-bit PPM, maxval=" & fields[3]
    inc i   # the single whitespace byte that terminates the header
    result.w = parseInt(fields[1])
    result.h = parseInt(fields[2])
    let need = result.w * result.h * 3
    doAssert raw.len - i == need,
      &"PPM payload is {raw.len - i} bytes, expected {need} for {result.w}x{result.h}"
    result.pixels = raw[i ..< i + need]

  proc sceneStats(f: CapturedFrame): SceneStats =
    ## Count and locate each of the scene's colours, and everything else.
    ##
    ## `other` is not a diagnostic — it is an assertion input. A frame
    ## that contains a fourth colour is a frame with something in it the
    ## shadow tree did not ask for, and the case below requires it to be
    ## zero.
    result.magenta = emptyStat()
    result.cyan = emptyStat()
    result.yellow = emptyStat()
    result.otherSample = 0
    result.otherX = -1
    result.otherY = -1
    for y in 0 ..< f.h:
      let rowBase = y * f.w * 3
      for x in 0 ..< f.w:
        let k = rowBase + x * 3
        let rgb = (uint32(uint8(f.pixels[k])) shl 16) or
                  (uint32(uint8(f.pixels[k + 1])) shl 8) or
                  uint32(uint8(f.pixels[k + 2]))
        case rgb
        of ColMagenta: observe(result.magenta, x, y)
        of ColCyan: observe(result.cyan, x, y)
        of ColYellow: observe(result.yellow, x, y)
        else:
          if result.other == 0:
            result.otherSample = rgb
            result.otherX = x
            result.otherY = y
          inc result.other

  # --- The watcher ----------------------------------------------------

  proc quitWhenCaptureDone(doneFile: string) {.thread.} =
    ## Wait for `wayland-capture-frame.sh` to signal that it is finished,
    ## then ask the GPUI event loop to return.
    ##
    ## This has to be a thread because the main thread is inside
    ## `gpui_launch`, i.e. inside `Application::run`, for exactly as long
    ## as the window exists. `gpui_quit` is an atomic store behind the
    ## FFI, so calling it from here is safe; the loop's own poller
    ## consumes it on the main thread, which is where GPUI requires
    ## `App::quit` to be called from.
    ##
    ## It does no process spawning and touches no shared Nim state — the
    ## path is passed by value through `createThread` — so it cannot
    ## interact with the event loop other than through that one flag.
    for _ in 0 ..< WatcherMaxTicks:
      if fileExists(doneFile):
        break
      sleep(WatcherTickMs)
    gpui_quit()

  suite "GUI - Windowed pixel capture (Wayland)":
    setup:
      gpui_reset_tree()
      gpui_reset_windows()
      resetCallbacks()

    test "window_paints_the_scene_the_shadow_tree_describes":
      require getEnv("WAYLAND_DISPLAY").len > 0
      require findExe("grim").len > 0
      let bashExe = findExe("bash")
      require bashExe.len > 0

      let captureScript = currentSourcePath().parentDir.parentDir /
        "scripts" / "wayland-capture-frame.sh"
      require fileExists(captureScript)

      let capDir = getTempDir() / ("isonim-gpui-capture-" & $getCurrentProcessId())
      createDir(capDir)
      let ppmPath = capDir / "frame.ppm"
      let donePath = ppmPath & ".done"
      let logPath = ppmPath & ".log"
      let blankPath = ppmPath & ".blank"
      removeFile(ppmPath)
      removeFile(donePath)
      removeFile(logPath)
      removeFile(blankPath)

      # Start the capture BEFORE the window: it polls, so it does not
      # matter that there is nothing to see yet, and starting it from the
      # main thread keeps all process spawning off the watcher thread.
      let helper = startProcess(bashExe,
        args = @[captureScript, ppmPath, $CaptureTimeoutSec],
        options = {poParentStreams})

      var watcher: Thread[string]
      createThread(watcher, quitWhenCaptureDone, donePath)

      # The backstop. Armed after `setup`'s `gpui_reset_windows`, which
      # clears it. Without this a broken watcher would hang the suite
      # instead of failing it.
      gpui_quit_after_ms(CaptureBackstopMs)

      let startedAt = epochTime()
      gpui_launch("IsoNim GPUI pixel capture".cstring, 800.0, 600.0,
                  buildPixelScene)
      let elapsedMs = (epochTime() - startedAt) * 1000.0

      joinThread(watcher)
      discard waitForExit(helper)
      close(helper)

      if fileExists(logPath):
        echo "--- wayland-capture-frame.sh log ---"
        echo readFile(logPath).strip()
        echo "------------------------------------"

      # Keep whatever was captured — the frame on success, the last
      # unsettled frame on timeout — next to the `--record` video, which
      # is the directory CI already uploads when this job fails. The
      # temp dir is deleted at the end of the case, and a pixel
      # assertion whose failure leaves no picture behind is a pixel
      # assertion nobody can act on from a CI log.
      let keepDir = getCurrentDir() / "target" / "test-recordings"
      createDir(keepDir)
      for suffix in ["", ".timeout", ".blank"]:
        let src =
          case suffix
          of "": ppmPath
          of ".blank": blankPath
          else: ppmPath & ".timeout.ppm"
        if fileExists(src):
          let kept = keepDir / ("pixel-capture" & suffix & ".ppm")
          copyFile(src, kept)
          echo "    frame kept at " & kept

      # `gpui_launch` returning at all is the first result: before
      # RS-M14b it could not, and this suite could not have existed.
      #
      # `require`, not `check`: `check` records the failure and carries
      # on, and everything below reads the file. A missing capture would
      # then surface as an unhandled IOError on top of the real verdict.
      if not fileExists(ppmPath):
        checkpoint("no frame was captured — see the log above")
      require fileExists(ppmPath)

      # And it returned because the capture finished, not because the
      # backstop expired. If this flips, the frame below is a picture of
      # something that timed out.
      check elapsedMs < float(CaptureBackstopMs)

      let frame = readPpm(ppmPath)
      let st = sceneStats(frame)

      # `echo`, not `checkpoint`: `checkpoint` is held back and printed
      # only when a case fails, and these four numbers are the evidence
      # that the case looked at a real frame. A green run that does not
      # say what it measured is indistinguishable from a green run that
      # measured nothing.
      echo &"    captured {frame.w}x{frame.h} in {elapsedMs.int} ms; " &
        &"magenta={st.magenta.count} cyan={st.cyan.count} " &
        &"yellow={st.yellow.count} other={st.other}"

      # Nothing on screen that the shadow tree did not put there.
      if st.other != 0:
        checkpoint(&"first unexpected colour 0x{st.otherSample.toHex(6)} " &
          &"at ({st.otherX},{st.otherY})")
      check st.other == 0

      # The root fills the window, and the window fills the output.
      check st.magenta.count ==
        frame.w * frame.h - SceneBoxAW * SceneBoxAH - SceneBoxBW * SceneBoxBH
      check st.magenta.x0 == 0
      check st.magenta.y0 == 0
      check st.magenta.x1 == frame.w - 1
      check st.magenta.y1 == frame.h - 1

      # Box A: first child of a row, so flush to the origin.
      check st.cyan.count == SceneBoxAW * SceneBoxAH
      check st.cyan.x0 == 0
      check st.cyan.y0 == 0
      check st.cyan.x1 == SceneBoxAW - 1
      check st.cyan.y1 == SceneBoxAH - 1

      # Box B: one box plus one gap to the right of the origin. This is
      # the assertion that `flex-direction: row` and `gap` reached the
      # renderer rather than merely reaching the render plan.
      check st.yellow.count == SceneBoxBW * SceneBoxBH
      check st.yellow.x0 == SceneBoxAW + SceneGap
      check st.yellow.y0 == 0
      check st.yellow.x1 == SceneBoxAW + SceneGap + SceneBoxBW - 1
      check st.yellow.y1 == SceneBoxBH - 1

      # --- THE BLANK CONTROL, FALSIFIED IN THIS RUN ---------------------
      #
      # Every assertion above is of the form "the frame contains N pixels
      # of colour C". On its own that is a positive half with no negative
      # beside it: it says what the frame HAS, and it would be equally
      # satisfied by a stored picture, a frame from a previous case, or
      # any screen that happened to hold those colours. What makes it a
      # claim about THIS window is that the SAME compositor, in the SAME
      # run, with no client attached, produced a frame on which those
      # assertions are all false — and until `wayland-capture-frame.sh`
      # retained that frame, there was no such frame to check against.
      #
      # `codetracer-specs/Testing/Verification-Harness-Traps.md` §7b: a
      # control you have never made fail is not a control. So the control
      # is made to fail here, on the three colour counts in the same
      # order, rather than described in a comment.
      require fileExists(blankPath)
      let blank = readPpm(blankPath)
      let bst = sceneStats(blank)
      echo &"    blank control {blank.w}x{blank.h}; " &
        &"magenta={bst.magenta.count} cyan={bst.cyan.count} " &
        &"yellow={bst.yellow.count} other={bst.other}"
      # The control is the same output, so the dimensions match and a
      # difference in the counts cannot be a difference in the frame size.
      check blank.w == frame.w
      check blank.h == frame.h
      # And the scene is absent from it, on all three colours.
      check bst.magenta.count == 0
      check bst.cyan.count == 0
      check bst.yellow.count == 0
      # Two-sided: "the scene's colours are absent" would also be true of
      # a zero-byte read, so the control has to account for every pixel it
      # claims to have. `other` is everything that is not one of the three.
      check bst.other == blank.w * blank.h

      removeDir(capDir)
