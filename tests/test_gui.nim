## GUI tests for isonim-gpui.
## Run under headless display: just test-gui-x11
## Or without display (render plan only): nim c -r tests/test_gui.nim
##
## Build & run:
##   LD_LIBRARY_PATH=rust/target/debug nim c -r --path:../isonim/src tests/test_gui.nim

import unittest
import std/[json, strutils]
import isonim_gpui/renderer
import isonim_gpui/bindings

# ============================================================================
# Helpers
# ============================================================================

proc getPlan(r: GpuiRenderer; node: GpuiElement): JsonNode =
  ## Build the render plan for a node and parse it as JSON.
  let jsonStr = r.renderPlanJson(node)
  check jsonStr.len > 0
  result = parseJson(jsonStr)

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
# Launch Integration Tests (no display server required)
# ============================================================================
#
# These use gpui_launch WITHOUT the gpui-backend feature (default).
# The launch function calls the root_builder callback, sets up the shadow
# tree, and returns immediately. This tests the full Nim->FFI->Rust->callback->tree
# pipeline.

suite "GUI - Launch Integration Tests":
  setup:
    gpui_reset_tree()
    gpui_reset_windows()
    resetCallbacks()

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
# GPUI Window Tests (require display server -- Xvfb or Wayland)
# ============================================================================

when defined(gpuiBackend):
  suite "GUI - GPUI Backend Compile Check":
    test "gpui_backend_feature_enabled":
      # When compiled with -d:gpuiBackend, the Rust shim should be built
      # with --features gpui-backend, which enables the actual GPUI rendering.
      # We can't test the blocking event loop directly, but we can verify
      # that window management works.
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
