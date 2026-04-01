## Integration tests for the full Nim -> FFI -> Rust -> render-plan pipeline (G3-G).
##
## These tests verify that:
## 1. GpuiRenderer creates elements that produce valid render plans
## 2. The render plan has correct GPUI element types (Div, TextContainer, TextNode)
## 3. Styles are mapped correctly through the render plan
## 4. Event handlers are present in the render plan
## 5. Render plan updates after reactive changes
##
## Unlike test_cross_renderer.nim (which reads back from the shadow tree),
## these tests query the render plan -- the intermediate representation that
## drives actual GPUI rendering.
##
## Build & run:
##   LD_LIBRARY_PATH=rust/target/debug nim c -r --path:../isonim/src tests/test_render_integration.nim

import unittest
import std/json

# IsoNim reactive core
import isonim/core/[signals, computation, owner]

# GpuiRenderer
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
# Test suites
# ============================================================================

suite "Render Plan - Basic Elements":
  setup:
    gpui_reset_tree()
    resetCallbacks()

  test "div maps to Div in render plan":
    let r = GpuiRenderer()
    let divEl = r.createElement("div")
    let plan = r.getPlan(divEl)
    check plan["kind"].getStr == "Div"
    check r.verifyRenderPlan(divEl)

  test "span maps to TextContainer in render plan":
    let r = GpuiRenderer()
    let span = r.createElement("span")
    let plan = r.getPlan(span)
    check plan["kind"].getStr == "TextContainer"
    check plan["tag"].getStr == "span"

  test "p maps to TextContainer in render plan":
    let r = GpuiRenderer()
    let p = r.createElement("p")
    let plan = r.getPlan(p)
    check plan["kind"].getStr == "TextContainer"
    check plan["tag"].getStr == "p"

  test "button maps to Div in render plan":
    let r = GpuiRenderer()
    let btn = r.createElement("button")
    let plan = r.getPlan(btn)
    check plan["kind"].getStr == "Div"

  test "text node maps to TextNode with text":
    let r = GpuiRenderer()
    let text = r.createTextNode("hello")
    let plan = r.getPlan(text)
    check plan["kind"].getStr == "TextNode"
    check plan["text"].getStr == "hello"

  test "HTML semantic tags map to Div":
    let r = GpuiRenderer()
    for tag in ["header", "footer", "article", "section", "nav", "aside"]:
      gpui_reset_tree()
      let elem = r.createElement(tag)
      let plan = r.getPlan(elem)
      check plan["kind"].getStr == "Div"

  test "nil node returns empty plan":
    let r = GpuiRenderer()
    let json = r.renderPlanJson(nil)
    check json == ""
    check not r.verifyRenderPlan(nil)
    check r.renderPlanElementCount(nil) == 0

suite "Render Plan - Tree Structure":
  setup:
    gpui_reset_tree()
    resetCallbacks()

  test "parent-child structure in render plan":
    let r = GpuiRenderer()
    let parent = r.createElement("div")
    let child1 = r.createElement("span")
    let child2 = r.createElement("span")
    r.appendChild(parent, child1)
    r.appendChild(parent, child2)

    let plan = r.getPlan(parent)
    check plan["kind"].getStr == "Div"
    check plan["children"].len == 2
    check plan["children"][0]["kind"].getStr == "TextContainer"
    check plan["children"][1]["kind"].getStr == "TextContainer"

  test "element count matches tree size":
    let r = GpuiRenderer()
    let root = r.createElement("div")
    let a = r.createElement("span")
    let b = r.createElement("span")
    let c = r.createTextNode("text")
    r.appendChild(a, c)
    r.appendChild(root, a)
    r.appendChild(root, b)

    check r.renderPlanElementCount(root) == 4

  test "deep nesting in render plan":
    let r = GpuiRenderer()
    let root = r.createElement("div")
    let inner = r.createElement("div")
    let span = r.createElement("span")
    let text = r.createTextNode("nested")
    r.appendChild(span, text)
    r.appendChild(inner, span)
    r.appendChild(root, inner)

    let plan = r.getPlan(root)
    check plan["children"][0]["children"][0]["children"][0]["text"].getStr == "nested"
    check r.renderPlanElementCount(root) == 4

suite "Render Plan - Styles":
  setup:
    gpui_reset_tree()
    resetCallbacks()

  test "CSS background-color maps to bg":
    let r = GpuiRenderer()
    let el = r.createElement("div")
    r.setStyle(el, "background-color", "red")
    let plan = r.getPlan(el)
    check plan["styles"]["bg"].getStr == "red"

  test "CSS width and height map to w and h":
    let r = GpuiRenderer()
    let el = r.createElement("div")
    r.setStyle(el, "width", "200px")
    r.setStyle(el, "height", "100px")
    let plan = r.getPlan(el)
    check plan["styles"]["w"].getStr == "200px"
    check plan["styles"]["h"].getStr == "100px"

  test "CSS flex-direction row maps to row":
    let r = GpuiRenderer()
    let el = r.createElement("div")
    r.setStyle(el, "flex-direction", "row")
    let plan = r.getPlan(el)
    check plan["styles"]["flex_direction"].getStr == "row"

  test "CSS padding maps to p":
    let r = GpuiRenderer()
    let el = r.createElement("div")
    r.setStyle(el, "padding", "10")
    let plan = r.getPlan(el)
    check plan["styles"]["p"].getStr == "10"

  test "CSS margin maps to m":
    let r = GpuiRenderer()
    let el = r.createElement("div")
    r.setStyle(el, "margin", "5")
    let plan = r.getPlan(el)
    check plan["styles"]["m"].getStr == "5"

  test "CSS font-size maps to text_size":
    let r = GpuiRenderer()
    let el = r.createElement("div")
    r.setStyle(el, "font-size", "16")
    let plan = r.getPlan(el)
    check plan["styles"]["text_size"].getStr == "16"

  test "CSS color maps to text_color":
    let r = GpuiRenderer()
    let el = r.createElement("div")
    r.setStyle(el, "color", "#333")
    let plan = r.getPlan(el)
    check plan["styles"]["text_color"].getStr == "#333"

  test "CSS border-radius maps to rounded":
    let r = GpuiRenderer()
    let el = r.createElement("div")
    r.setStyle(el, "border-radius", "8")
    let plan = r.getPlan(el)
    check plan["styles"]["rounded"].getStr == "8"

  test "CSS gap maps to gap":
    let r = GpuiRenderer()
    let el = r.createElement("div")
    r.setStyle(el, "gap", "5")
    let plan = r.getPlan(el)
    check plan["styles"]["gap"].getStr == "5"

suite "Render Plan - Event Handlers":
  setup:
    gpui_reset_tree()
    resetCallbacks()

  test "click handler shows in render plan":
    let r = GpuiRenderer()
    let btn = r.createElement("button")
    r.addEventListener(btn, "click", proc() = discard)
    let plan = r.getPlan(btn)
    check plan["has_click_handler"].getBool == true
    check plan["has_input_handler"].getBool == false

  test "no handlers by default":
    let r = GpuiRenderer()
    let el = r.createElement("div")
    let plan = r.getPlan(el)
    check plan["has_click_handler"].getBool == false
    check plan["has_input_handler"].getBool == false
    check plan["event_names"].len == 0

  test "event handlers preserved in children":
    let r = GpuiRenderer()
    let root = r.createElement("div")
    let btn = r.createElement("button")
    r.addEventListener(btn, "click", proc() = discard)
    r.appendChild(root, btn)

    let plan = r.getPlan(root)
    check plan["has_click_handler"].getBool == false
    check plan["children"][0]["has_click_handler"].getBool == true

  test "click dispatch works and handler fires":
    var clicked = false
    let r = GpuiRenderer()
    let btn = r.createElement("button")
    r.addEventListener(btn, "click", proc() = clicked = true)
    let plan = r.getPlan(btn)
    check plan["has_click_handler"].getBool == true

    fireEvent(btn, "click")
    check clicked == true

suite "Render Plan - Counter App":
  setup:
    gpui_reset_tree()
    resetCallbacks()

  test "counter app render plan structure":
    ## Build a counter app with GpuiRenderer and verify the render plan.
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      var count = createSignal(0)

      let container = r.createElement("div")
      let label = r.createTextNode("")
      let incBtn = r.createElement("button")
      let decBtn = r.createElement("button")

      r.appendChild(incBtn, r.createTextNode("+"))
      r.appendChild(decBtn, r.createTextNode("-"))
      r.appendChild(container, label)
      r.appendChild(container, incBtn)
      r.appendChild(container, decBtn)

      r.addEventListener(incBtn, "click", proc() =
        count.val = count.val + 1
      )
      r.addEventListener(decBtn, "click", proc() =
        count.val = count.val - 1
      )

      createRenderEffect proc() =
        r.setTextContent(label, "Count: " & $count.val)

      # Verify render plan structure
      let plan = r.getPlan(container)
      check plan["kind"].getStr == "Div"  # div -> Div
      check plan["children"].len == 3     # label + incBtn + decBtn
      check r.verifyRenderPlan(container)

      # label (text node) -> TextNode
      check plan["children"][0]["kind"].getStr == "TextNode"
      check plan["children"][0]["text"].getStr == "Count: 0"

      # inc button -> Div with click handler
      check plan["children"][1]["kind"].getStr == "Div"
      check plan["children"][1]["has_click_handler"].getBool == true

      # dec button -> Div with click handler
      check plan["children"][2]["kind"].getStr == "Div"
      check plan["children"][2]["has_click_handler"].getBool == true

      dispose()

  test "counter app render plan updates after click":
    ## Verify that reactive changes update the shadow tree, and the rebuilt
    ## render plan reflects those changes.
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      var count = createSignal(0)

      let container = r.createElement("div")
      let label = r.createTextNode("")
      let incBtn = r.createElement("button")
      r.appendChild(incBtn, r.createTextNode("+"))
      r.appendChild(container, label)
      r.appendChild(container, incBtn)

      r.addEventListener(incBtn, "click", proc() =
        count.val = count.val + 1
      )

      createRenderEffect proc() =
        r.setTextContent(label, "Count: " & $count.val)

      # Initial state
      var plan = r.getPlan(container)
      check plan["children"][0]["text"].getStr == "Count: 0"

      # Click increment
      fireEvent(incBtn, "click")
      plan = r.getPlan(container)
      check plan["children"][0]["text"].getStr == "Count: 1"

      # Click again
      fireEvent(incBtn, "click")
      fireEvent(incBtn, "click")
      plan = r.getPlan(container)
      check plan["children"][0]["text"].getStr == "Count: 3"

      dispose()

suite "Render Plan - Task Manager Demo":
  setup:
    gpui_reset_tree()
    resetCallbacks()

  test "task manager render plan structure":
    ## Build a simplified task manager and verify its render plan has the
    ## correct GPUI element types.
    let r = GpuiRenderer()

    let app = r.createElement("div")
    let header = r.createElement("header")
    let title = r.createElement("h1")
    r.setTextContent(title, "Task Manager")
    r.appendChild(header, title)
    r.appendChild(app, header)

    let inputArea = r.createElement("div")
    let addBtn = r.createElement("button")
    r.setTextContent(addBtn, "Add")
    r.addEventListener(addBtn, "click", proc() = discard)
    r.appendChild(inputArea, addBtn)
    r.appendChild(app, inputArea)

    let taskList = r.createElement("ul")
    for task in ["Design API", "Write tests", "Deploy"]:
      let li = r.createElement("li")
      let span = r.createElement("span")
      r.setTextContent(span, task)
      r.appendChild(li, span)
      r.appendChild(taskList, li)
    r.appendChild(app, taskList)

    let footer = r.createElement("footer")
    let countSpan = r.createElement("span")
    r.setTextContent(countSpan, "3 tasks")
    r.appendChild(footer, countSpan)
    r.appendChild(app, footer)

    # Verify the render plan
    let plan = r.getPlan(app)
    check plan["kind"].getStr == "Div"
    check plan["children"].len == 4  # header, inputArea, taskList, footer

    # header -> Div (mapped from "header")
    check plan["children"][0]["kind"].getStr == "Div"
    # h1 -> TextContainer
    check plan["children"][0]["children"][0]["kind"].getStr == "TextContainer"

    # input area -> Div
    check plan["children"][1]["kind"].getStr == "Div"
    # add button -> Div with click handler
    check plan["children"][1]["children"][0]["has_click_handler"].getBool == true

    # task list (ul) -> Div with 3 children
    let taskListPlan = plan["children"][2]
    check taskListPlan["kind"].getStr == "Div"
    check taskListPlan["children"].len == 3

    # Each li -> Div, each span -> TextContainer
    for i in 0 ..< 3:
      check taskListPlan["children"][i]["kind"].getStr == "Div"
      check taskListPlan["children"][i]["children"][0]["kind"].getStr == "TextContainer"

    # footer -> Div, span -> TextContainer
    check plan["children"][3]["kind"].getStr == "Div"
    check plan["children"][3]["children"][0]["kind"].getStr == "TextContainer"

    # Total element count
    check r.renderPlanElementCount(app) > 10
    check r.verifyRenderPlan(app)

  test "task manager render plan updates after adding task":
    ## Verify that adding a task to the tree is reflected in the render plan.
    let r = GpuiRenderer()

    let taskList = r.createElement("ul")
    let li1 = r.createElement("li")
    let span1 = r.createElement("span")
    r.setTextContent(span1, "Task 1")
    r.appendChild(li1, span1)
    r.appendChild(taskList, li1)

    var plan = r.getPlan(taskList)
    check plan["children"].len == 1

    # Add another task
    let li2 = r.createElement("li")
    let span2 = r.createElement("span")
    r.setTextContent(span2, "Task 2")
    r.appendChild(li2, span2)
    r.appendChild(taskList, li2)

    plan = r.getPlan(taskList)
    check plan["children"].len == 2

  test "render plan with styled task items":
    ## Verify styles propagate through task items in the render plan.
    let r = GpuiRenderer()

    let li = r.createElement("li")
    r.setStyle(li, "background-color", "#f5f5f5")
    r.setStyle(li, "padding", "8")
    r.setStyle(li, "border-radius", "4")

    let span = r.createElement("span")
    r.setStyle(span, "color", "#333")
    r.setStyle(span, "font-size", "14")
    r.setTextContent(span, "Styled task")
    r.appendChild(li, span)

    let plan = r.getPlan(li)
    check plan["styles"]["bg"].getStr == "#f5f5f5"
    check plan["styles"]["p"].getStr == "8"
    check plan["styles"]["rounded"].getStr == "4"

    let spanPlan = plan["children"][0]
    check spanPlan["styles"]["text_color"].getStr == "#333"
    check spanPlan["styles"]["text_size"].getStr == "14"

suite "Render Plan - Reactive Updates":
  setup:
    gpui_reset_tree()
    resetCallbacks()

  test "render plan reflects signal-driven text change":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      var text = createSignal("initial")
      let label = r.createTextNode("")

      createRenderEffect proc() =
        r.setTextContent(label, text.val)

      var plan = r.getPlan(label)
      check plan["text"].getStr == "initial"

      text.val = "updated"
      plan = r.getPlan(label)
      check plan["text"].getStr == "updated"

      text.val = "final"
      plan = r.getPlan(label)
      check plan["text"].getStr == "final"

      dispose()

  test "render plan reflects dynamic child addition":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      var items = createSignal(newSeq[string]())
      let container = r.createElement("div")

      proc rebuildList() =
        # Clear children (simplified -- in real code use reconciliation)
        while childCount(container) > 0:
          let child = r.firstChild(container)
          if child != nil:
            r.removeChild(container, child)
          else:
            break
        for item in items.val:
          let span = r.createElement("span")
          r.setTextContent(span, item)
          r.appendChild(container, span)

      createRenderEffect proc() =
        rebuildList()

      var plan = r.getPlan(container)
      check plan["children"].len == 0

      items.val = @["alpha", "beta"]
      plan = r.getPlan(container)
      check plan["children"].len == 2

      items.val = @["alpha", "beta", "gamma"]
      plan = r.getPlan(container)
      check plan["children"].len == 3

      dispose()

  test "render plan reflects style change via signal":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      var bg = createSignal("red")
      let el = r.createElement("div")

      createRenderEffect proc() =
        r.setStyle(el, "background-color", bg.val)

      var plan = r.getPlan(el)
      check plan["styles"]["bg"].getStr == "red"

      bg.val = "blue"
      plan = r.getPlan(el)
      check plan["styles"]["bg"].getStr == "blue"

      dispose()
