## Cross-renderer tests for GpuiRenderer (M5).
##
## Ports the generic createCounter and createTaskList test components from
## isonim's test_native_renderer.nim and verifies that GpuiRenderer produces
## the same behavior as MockRenderer and TerminalRenderer.
##
## These tests link against the Rust shim and require it to be built first:
##   just rust-build
##
## Run with:
##   LD_LIBRARY_PATH=rust/target/debug nim c -r --path:../isonim/src tests/test_cross_renderer.nim

import unittest
import std/tables

# IsoNim renderers
import isonim/testing/mock_dom
import isonim/renderers/terminal

# IsoNim reactive core
import isonim/core/[signals, computation, owner]

# GpuiRenderer
import isonim_gpui/renderer
import isonim_gpui/bindings

# ============================================================================
# Generic test components (work with any RendererBackend)
# ============================================================================
# These are the same components from isonim/tests/test_native_renderer.nim,
# duplicated here to avoid depending on isonim's test files at import time.

proc createCounter[R, N](renderer: R): N =
  ## A counter component that works with any RendererBackend.
  var count = createSignal(0)
  let container = renderer.createElement("div")
  let label = renderer.createTextNode("")
  let incBtn = renderer.createElement("button")
  let decBtn = renderer.createElement("button")

  renderer.appendChild(incBtn, renderer.createTextNode("+"))
  renderer.appendChild(decBtn, renderer.createTextNode("-"))
  renderer.appendChild(container, label)
  renderer.appendChild(container, incBtn)
  renderer.appendChild(container, decBtn)

  renderer.addEventListener(incBtn, "click", proc() =
    count.val = count.val + 1
  )
  renderer.addEventListener(decBtn, "click", proc() =
    count.val = count.val - 1
  )

  createRenderEffect proc() =
    renderer.setTextContent(label, "Count: " & $count.val)

  return container

proc createTaskList[R, N](renderer: R; items: seq[string]): N =
  ## A list component that works with any RendererBackend.
  let ul = renderer.createElement("ul")
  for item in items:
    let li = renderer.createElement("li")
    let span = renderer.createElement("span")
    renderer.setTextContent(span, item)
    renderer.appendChild(li, span)
    renderer.appendChild(ul, li)
  return ul

# ============================================================================
# GpuiRenderer-specific tree inspection helpers
# ============================================================================

proc gpuiChildCount(node: GpuiElement): int =
  renderer.childCount(node)

proc gpuiTextContent(node: GpuiElement): string =
  renderer.textContent(node)

# ============================================================================
# Test suites
# ============================================================================

suite "GpuiRenderer - Basic Operations":
  setup:
    gpui_reset_tree()
    resetCallbacks()

  test "createElement returns non-null":
    let r = GpuiRenderer()
    let node = r.createElement("div")
    check node != nil

  test "createTextNode returns non-null":
    let r = GpuiRenderer()
    let node = r.createTextNode("hello")
    check node != nil

  test "appendChild builds tree":
    let r = GpuiRenderer()
    let parent = r.createElement("div")
    let child = r.createElement("span")
    r.appendChild(parent, child)
    check gpuiChildCount(parent) == 1
    check r.firstChild(parent) != nil

  test "removeChild removes from tree":
    let r = GpuiRenderer()
    let parent = r.createElement("div")
    let child = r.createElement("span")
    r.appendChild(parent, child)
    r.removeChild(parent, child)
    check gpuiChildCount(parent) == 0

  test "insertBefore inserts at correct position":
    let r = GpuiRenderer()
    let parent = r.createElement("div")
    let a = r.createElement("span")
    let b = r.createElement("span")
    let c = r.createElement("span")
    r.appendChild(parent, a)
    r.appendChild(parent, c)
    r.insertBefore(parent, b, c)
    check gpuiChildCount(parent) == 3

  test "tree navigation works":
    let r = GpuiRenderer()
    let parent = r.createElement("div")
    let a = r.createElement("span")
    let b = r.createElement("span")
    r.appendChild(parent, a)
    r.appendChild(parent, b)
    check r.firstChild(parent) != nil
    check r.nextSibling(a) != nil
    check r.parentNode(a) != nil

  test "setText and getText round-trip on text node":
    let r = GpuiRenderer()
    let node = r.createTextNode("initial")
    check gpuiTextContent(node) == "initial"
    r.setTextContent(node, "updated")
    check gpuiTextContent(node) == "updated"

  test "setAttribute and getAttribute round-trip":
    let r = GpuiRenderer()
    let node = r.createElement("div")
    r.setAttribute(node, "class", "container")
    check renderer.getAttribute(node, "class") == "container"

suite "GpuiRenderer - Reactive Counter":
  setup:
    gpui_reset_tree()
    resetCallbacks()

  test "reactive counter works with GpuiRenderer":
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      let counter = createCounter[GpuiRenderer, GpuiElement](r)

      check gpuiTextContent(counter) == "Count: 0+-"

      # Click increment (child index 1 = incBtn)
      let incBtn = nthChild(counter, 1)
      check incBtn != nil
      fireEvent(incBtn, "click")
      check gpuiTextContent(counter) == "Count: 1+-"

      fireEvent(incBtn, "click")
      fireEvent(incBtn, "click")
      check gpuiTextContent(counter) == "Count: 3+-"

      # Click decrement (child index 2 = decBtn)
      let decBtn = nthChild(counter, 2)
      check decBtn != nil
      fireEvent(decBtn, "click")
      check gpuiTextContent(counter) == "Count: 2+-"

      dispose()

suite "GpuiRenderer - Task List":
  setup:
    gpui_reset_tree()
    resetCallbacks()

  test "task list renders with GpuiRenderer":
    let r = GpuiRenderer()
    let list = createTaskList[GpuiRenderer, GpuiElement](r,
      @["Buy groceries", "Write code", "Test app"])

    check gpuiChildCount(list) == 3
    let child0 = nthChild(list, 0)
    let child1 = nthChild(list, 1)
    let child2 = nthChild(list, 2)
    check gpuiTextContent(child0) == "Buy groceries"
    check gpuiTextContent(child1) == "Write code"
    check gpuiTextContent(child2) == "Test app"

suite "Cross-Renderer Compatibility":
  setup:
    gpui_reset_tree()
    resetCallbacks()

  test "same counter component works across GpuiRenderer and MockRenderer":
    createRoot proc(dispose: proc()) =
      let gr = GpuiRenderer()
      let mr = MockRenderer()

      let gpuiCounter = createCounter[GpuiRenderer, GpuiElement](gr)
      let mockCounter = createCounter[MockRenderer, MockNode](mr)

      # Both start at 0
      check gpuiTextContent(gpuiCounter) == "Count: 0+-"
      check mock_dom.textContent(mockCounter) == "Count: 0+-"

      # Same child count
      check gpuiChildCount(gpuiCounter) == mockCounter.children.len

      # Increment GPUI counter
      let gpuiIncBtn = nthChild(gpuiCounter, 1)
      fireEvent(gpuiIncBtn, "click")
      check gpuiTextContent(gpuiCounter) == "Count: 1+-"

      # Increment Mock counter
      mockCounter.children[1].fireEvent("click")
      check mock_dom.textContent(mockCounter) == "Count: 1+-"

      # Both produce same text after increment
      check gpuiTextContent(gpuiCounter) == mock_dom.textContent(mockCounter)

      dispose()

  test "same counter component works across GpuiRenderer and TerminalRenderer":
    createRoot proc(dispose: proc()) =
      let gr = GpuiRenderer()
      let tr = TerminalRenderer()

      let gpuiCounter = createCounter[GpuiRenderer, GpuiElement](gr)
      let terminalCounter = createCounter[TerminalRenderer, TerminalNode](tr)

      # Both start at 0
      check gpuiTextContent(gpuiCounter) == "Count: 0+-"
      check terminal.textContent(terminalCounter) == "Count: 0+-"

      # Increment both
      let gpuiIncBtn = nthChild(gpuiCounter, 1)
      fireEvent(gpuiIncBtn, "click")
      terminalCounter.children[1].fireEvent("click")

      # Both at 1
      check gpuiTextContent(gpuiCounter) == "Count: 1+-"
      check terminal.textContent(terminalCounter) == "Count: 1+-"
      check gpuiTextContent(gpuiCounter) == terminal.textContent(terminalCounter)

      # Same child count
      check gpuiChildCount(gpuiCounter) == terminalCounter.children.len

      dispose()

  test "same task list works across all renderers":
    let gr = GpuiRenderer()
    let tr = TerminalRenderer()
    let mr = MockRenderer()
    let items = @["Alpha", "Beta", "Gamma"]

    let gList = createTaskList[GpuiRenderer, GpuiElement](gr, items)
    let tList = createTaskList[TerminalRenderer, TerminalNode](tr, items)
    let mList = createTaskList[MockRenderer, MockNode](mr, items)

    # Same child count
    check gpuiChildCount(gList) == 3
    check tList.children.len == 3
    check mList.children.len == 3

    # Same text content per item
    for i in 0..2:
      let gChild = nthChild(gList, i)
      check gpuiTextContent(gChild) == items[i]
      check terminal.textContent(tList.children[i]) == items[i]
      check mock_dom.textContent(mList.children[i]) == items[i]

suite "GpuiRenderer - Task Manager Demo":
  setup:
    gpui_reset_tree()
    resetCallbacks()

  test "task manager app structure":
    ## Simplified version of the demo app to verify GpuiRenderer can
    ## build a realistic component tree.
    let r = GpuiRenderer()

    # App root
    let app = r.createElement("div")
    r.setAttribute(app, "class", "app")

    # Header
    let header = r.createElement("header")
    let title = r.createElement("h1")
    r.setTextContent(title, "Task Manager")
    r.appendChild(header, title)
    r.appendChild(app, header)

    # Input area
    let inputArea = r.createElement("div")
    r.setAttribute(inputArea, "class", "input-area")
    let input = r.createElement("input")
    r.setAttribute(input, "placeholder", "New task...")
    let addBtn = r.createElement("button")
    r.setTextContent(addBtn, "Add")
    r.appendChild(inputArea, input)
    r.appendChild(inputArea, addBtn)
    r.appendChild(app, inputArea)

    # Task list
    let taskList = r.createElement("ul")
    r.setAttribute(taskList, "class", "task-list")

    var tasks = @["Design API", "Write tests", "Deploy"]
    for task in tasks:
      let li = r.createElement("li")
      let checkbox = r.createElement("input")
      r.setAttribute(checkbox, "type", "checkbox")
      let label = r.createElement("span")
      r.setTextContent(label, task)
      r.appendChild(li, checkbox)
      r.appendChild(li, label)
      r.appendChild(taskList, li)

    r.appendChild(app, taskList)

    # Footer
    let footer = r.createElement("footer")
    let count = r.createElement("span")
    r.setTextContent(count, "3 tasks")
    r.appendChild(footer, count)
    r.appendChild(app, footer)

    # Verify structure
    check gpuiChildCount(app) == 4 # header, inputArea, taskList, footer
    check gpuiChildCount(taskList) == 3 # 3 task items
    check renderer.getAttribute(app, "class") == "app"
    check renderer.getAttribute(taskList, "class") == "task-list"
    check gpuiTextContent(footer) == "3 tasks"

    # Verify task text
    for i in 0..2:
      let li = nthChild(taskList, i)
      check gpuiTextContent(li) == tasks[i]

  test "task manager dynamic updates":
    ## Verify that reactive updates work in a task-manager-like component.
    createRoot proc(dispose: proc()) =
      let r = GpuiRenderer()
      var taskCount = createSignal(0)

      let app = r.createElement("div")
      let counter = r.createElement("span")
      r.appendChild(app, counter)

      createRenderEffect proc() =
        r.setTextContent(counter, $taskCount.val & " tasks")

      check gpuiTextContent(counter) == "0 tasks"

      taskCount.val = 1
      check gpuiTextContent(counter) == "1 tasks"

      taskCount.val = 5
      check gpuiTextContent(counter) == "5 tasks"

      dispose()
