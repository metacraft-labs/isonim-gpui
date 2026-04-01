## Structural comparison tests for isonim-gpui (G4).
##
## These tests build the task manager demo structure and verify the render plan
## has the correct GPUI element types, styles, event handlers, and element counts.
## This replaces screenshot comparison with practical structural verification.
##
## Build & run:
##   LD_LIBRARY_PATH=rust/target/debug nim c -r --path:../isonim/src tests/test_structural_comparison.nim

import unittest
import std/[json, strutils]

# IsoNim reactive core
import isonim/core/[signals, computation, owner]

# GpuiRenderer
import isonim_gpui/renderer
import isonim_gpui/bindings

# ============================================================================
# Helpers
# ============================================================================

proc getPlan(r: GpuiRenderer; node: GpuiElement): JsonNode =
  let jsonStr = r.renderPlanJson(node)
  check jsonStr.len > 0
  result = parseJson(jsonStr)

# ============================================================================
# Test suites
# ============================================================================

suite "Structural Verification - Task Manager":
  setup:
    gpui_reset_tree()
    resetCallbacks()

  test "demo_app_render_plan_structure":
    ## Build the task manager and verify render plan has correct GPUI element types
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()

      # Build a task manager structure
      let root = r.createElement("div")
      r.setAttribute(root, "class", "app")

      let header = r.createElement("header")
      let h1 = r.createElement("h1")
      let title = r.createTextNode("Task Manager")
      r.appendChild(h1, title)
      r.appendChild(header, h1)
      r.appendChild(root, header)

      let taskList = r.createElement("ul")
      r.setAttribute(taskList, "class", "task-list")

      # Add two tasks
      for taskText in ["Learn IsoNim", "Build demo app"]:
        let li = r.createElement("li")
        let span = r.createElement("span")
        let text = r.createTextNode(taskText)
        r.appendChild(span, text)
        r.appendChild(li, span)
        let btn = r.createElement("button")
        let btnText = r.createTextNode("Delete")
        r.appendChild(btn, btnText)
        r.addEventListener(btn, "click", proc() = discard)
        r.appendChild(li, btn)
        r.appendChild(taskList, li)

      r.appendChild(root, taskList)

      # Verify render plan
      let plan = r.getPlan(root)

      # Root is Div (div)
      check plan["kind"].getStr() == "Div"

      # Header child (header -> Div)
      let headerPlan = plan["children"][0]
      check headerPlan["kind"].getStr() == "Div"

      # h1 is TextContainer
      let h1Plan = headerPlan["children"][0]
      check h1Plan["kind"].getStr() == "TextContainer"

      # Task list (ul -> Div)
      let listPlan = plan["children"][1]
      check listPlan["kind"].getStr() == "Div"
      check listPlan["children"].len == 2  # two tasks

      # Each task item has a span (TextContainer) and button (Div with click)
      for i in 0 ..< 2:
        let item = listPlan["children"][i]
        check item["kind"].getStr() == "Div"  # li -> Div
        # span child -> TextContainer
        check item["children"][0]["kind"].getStr() == "TextContainer"
        # button child -> Div with click handler
        check item["children"][1]["kind"].getStr() == "Div"
        check item["children"][1]["has_click_handler"].getBool() == true

      # Element count
      let count = r.renderPlanElementCount(root)
      check count > 0

      # Validity
      check r.verifyRenderPlan(root)

      dispose()

  test "demo_app_styles_in_render_plan":
    ## Verify CSS styles are correctly mapped in the render plan
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let root = r.createElement("div")
      r.setStyle(root, "background-color", "blue")
      r.setStyle(root, "width", "400px")
      r.setStyle(root, "flex-direction", "column")
      r.setStyle(root, "padding", "16px")

      let plan = r.getPlan(root)
      let styles = plan["styles"]

      check styles["bg"].getStr() == "blue"
      check styles["w"].getStr() == "400px"
      # flex-direction: column -> Nim maps to "col" -> Rust normalizes to "column"
      check styles["flex_direction"].getStr() == "column"
      check styles["p"].getStr() == "16px"

      dispose()

  test "demo_app_event_handlers_in_render_plan":
    ## Verify click handlers appear in render plan
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let btn = r.createElement("button")
      let text = r.createTextNode("Click me")
      r.appendChild(btn, text)
      r.addEventListener(btn, "click", proc() = discard)

      let plan = r.getPlan(btn)
      check plan["has_click_handler"].getBool() == true

      dispose()

  test "render_plan_element_count_matches":
    ## Element count from FFI matches manual count
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let root = r.createElement("div")
      for i in 0 ..< 5:
        let child = r.createElement("span")
        let text = r.createTextNode("Item " & $i)
        r.appendChild(child, text)
        r.appendChild(root, child)

      let count = r.renderPlanElementCount(root)
      # 1 root + 5 spans + 5 texts = 11
      check count == 11

      dispose()

  test "render_plan_validity_check":
    ## Render plan validation passes for valid tree
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let root = r.createElement("div")
      let child = r.createElement("span")
      r.appendChild(root, child)

      check r.verifyRenderPlan(root) == true

      dispose()

  test "structural_comparison_after_mutation":
    ## Render plan changes after tree mutation
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let root = r.createElement("ul")

      let countBefore = r.renderPlanElementCount(root)
      check countBefore == 1

      let item = r.createElement("li")
      let text = r.createTextNode("New item")
      r.appendChild(item, text)
      r.appendChild(root, item)

      let countAfter = r.renderPlanElementCount(root)
      check countAfter == 3  # root + li + text

      dispose()
