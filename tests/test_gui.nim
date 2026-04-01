## GUI tests for isonim-gpui.
## Run under headless display: just test-gui-x11
## Or without display (render plan only): nim c -r tests/test_gui.nim
##
## Build & run:
##   LD_LIBRARY_PATH=rust/target/debug nim c -r --path:../isonim/src tests/test_gui.nim

import unittest
import std/json
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
# GPUI Window Tests (require display server -- Xvfb or Wayland)
# ============================================================================

when defined(gpuiBackend):
  suite "GUI - GPUI Window Tests":
    # These tests require a display server (Xvfb or Wayland)
    # Run with: just test-gui-x11

    test "window_creation_smoke_test":
      # Just verify we can call the window creation FFI without crashing
      # The actual window will appear briefly under Xvfb
      skip()  # TODO: implement when gpui_app.rs launch is available via FFI
