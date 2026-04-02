## Comprehensive tests for the GpuiRenderer.
##
## Tests tag mapping, style mapping, attribute mapping, event callback bridge,
## tree operations, and window management. These are compile-time and unit
## tests that exercise the Nim-side logic.
##
## To run with the Rust shim linked (full integration test):
##   nim c -r --nimcache:nimcache/test_renderer tests/test_renderer.nim
##
## To verify compilation only (no Rust shim needed):
##   nim check --nimcache:nimcache/test_renderer tests/test_renderer.nim

import isonim_gpui/renderer
import isonim_gpui/bindings
import isonim_gpui/window

# ===========================================================================
# 1. Compile-time concept conformance
# ===========================================================================

static:
  var r: GpuiRenderer
  var e: GpuiElement

  # All 13 RendererBackend procs must type-check
  assert compiles(r.createElement("div"))
  assert compiles(r.createTextNode("hello"))
  assert compiles(r.appendChild(e, e))
  assert compiles(r.insertBefore(e, e, e))
  assert compiles(r.removeChild(e, e))
  assert compiles(r.setAttribute(e, "class", "container"))
  assert compiles(r.removeAttribute(e, "class"))
  assert compiles(r.setTextContent(e, "text"))
  assert compiles(r.setStyle(e, "color", "red"))
  assert compiles(r.addEventListener(e, "click", proc() = discard))
  assert compiles(r.firstChild(e))
  assert compiles(r.nextSibling(e))
  assert compiles(r.parentNode(e))

  # Return types
  assert r.createElement("") is GpuiElement
  assert r.createTextNode("") is GpuiElement
  assert r.firstChild(e) is GpuiElement
  assert r.nextSibling(e) is GpuiElement
  assert r.parentNode(e) is GpuiElement

echo "test_renderer: compile-time concept check passed"

# ===========================================================================
# 2. Tag mapping tests (compile-time)
# ===========================================================================

static:
  var r: GpuiRenderer
  # All common HTML tags should be accepted
  assert compiles(r.createElement("div"))
  assert compiles(r.createElement("span"))
  assert compiles(r.createElement("button"))
  assert compiles(r.createElement("input"))
  assert compiles(r.createElement("p"))
  assert compiles(r.createElement("h1"))
  assert compiles(r.createElement("ul"))
  assert compiles(r.createElement("li"))
  assert compiles(r.createElement("img"))
  assert compiles(r.createElement("form"))
  assert compiles(r.createElement("section"))
  assert compiles(r.createElement("article"))
  assert compiles(r.createElement("nav"))
  assert compiles(r.createElement("header"))
  assert compiles(r.createElement("footer"))
  assert compiles(r.createElement("textarea"))
  assert compiles(r.createElement("select"))
  assert compiles(r.createElement("ol"))
  assert compiles(r.createElement("svg"))
  # GPUI-native tags should also work (pass-through)
  assert compiles(r.createElement("div"))
  assert compiles(r.createElement("text"))
  assert compiles(r.createElement("img"))

echo "test_renderer: tag mapping compile check passed"

# ===========================================================================
# 3. Style mapping tests (compile-time)
# ===========================================================================

static:
  var r: GpuiRenderer
  var e: GpuiElement
  # CSS properties should be accepted
  assert compiles(r.setStyle(e, "background-color", "red"))
  assert compiles(r.setStyle(e, "color", "#333"))
  assert compiles(r.setStyle(e, "font-size", "16"))
  assert compiles(r.setStyle(e, "flex-direction", "row"))
  assert compiles(r.setStyle(e, "width", "100"))
  assert compiles(r.setStyle(e, "height", "50"))
  assert compiles(r.setStyle(e, "padding", "10"))
  assert compiles(r.setStyle(e, "margin", "5"))
  assert compiles(r.setStyle(e, "border-radius", "8"))
  assert compiles(r.setStyle(e, "align-items", "center"))
  assert compiles(r.setStyle(e, "justify-content", "space-between"))
  assert compiles(r.setStyle(e, "gap", "10"))
  assert compiles(r.setStyle(e, "overflow", "hidden"))
  assert compiles(r.setStyle(e, "opacity", "0.5"))
  assert compiles(r.setStyle(e, "cursor", "pointer"))
  assert compiles(r.setStyle(e, "box-shadow", "0 2px 4px rgba(0,0,0,0.1)"))
  assert compiles(r.setStyle(e, "border-color", "#ccc"))
  assert compiles(r.setStyle(e, "border-width", "1"))
  assert compiles(r.setStyle(e, "min-width", "100"))
  assert compiles(r.setStyle(e, "max-width", "800"))
  assert compiles(r.setStyle(e, "min-height", "50"))
  assert compiles(r.setStyle(e, "max-height", "600"))
  assert compiles(r.setStyle(e, "flex-grow", "1"))
  assert compiles(r.setStyle(e, "flex-shrink", "0"))
  assert compiles(r.setStyle(e, "flex-basis", "auto"))
  # GPUI-native properties should also work (pass-through)
  assert compiles(r.setStyle(e, "bg", "rgb(255,0,0)"))
  assert compiles(r.setStyle(e, "text_color", "#333"))
  assert compiles(r.setStyle(e, "text_size", "16"))

echo "test_renderer: style mapping compile check passed"

# ===========================================================================
# 4. Attribute mapping tests (compile-time)
# ===========================================================================

static:
  var r: GpuiRenderer
  var e: GpuiElement
  assert compiles(r.setAttribute(e, "class", "container"))
  assert compiles(r.setAttribute(e, "id", "main"))
  assert compiles(r.setAttribute(e, "disabled", ""))
  assert compiles(r.setAttribute(e, "placeholder", "Enter text"))
  assert compiles(r.setAttribute(e, "value", "hello"))
  assert compiles(r.setAttribute(e, "src", "image.png"))
  assert compiles(r.setAttribute(e, "alt", "description"))
  assert compiles(r.setAttribute(e, "title", "tooltip"))
  assert compiles(r.setAttribute(e, "href", "https://example.com"))
  assert compiles(r.setAttribute(e, "type", "text"))
  assert compiles(r.setAttribute(e, "name", "field"))
  assert compiles(r.removeAttribute(e, "disabled"))
  assert compiles(r.removeAttribute(e, "class"))

echo "test_renderer: attribute mapping compile check passed"

# ===========================================================================
# 5. Event callback bridge tests (compile-time)
# ===========================================================================

static:
  var r: GpuiRenderer
  var e: GpuiElement
  # Closures with captured variables should be accepted
  var counter = 0
  assert compiles(r.addEventListener(e, "click", proc() = counter += 1))
  assert compiles(r.addEventListener(e, "input", proc() = discard))
  assert compiles(r.addEventListener(e, "change", proc() = discard))
  assert compiles(r.addEventListener(e, "mousedown", proc() = discard))
  assert compiles(r.addEventListener(e, "mouseup", proc() = discard))
  assert compiles(r.addEventListener(e, "keydown", proc() = discard))

echo "test_renderer: event callback bridge compile check passed"

# ===========================================================================
# 6. Callback registry unit tests (via dispatcher round-trip through Rust shim)
# ===========================================================================

block callbackRegistryTest:
  gpui_reset_tree()
  resetCallbacks()

  var r: GpuiRenderer
  let btn = r.createElement("button")

  var counter = 0
  r.addEventListener(btn, "click", proc() = counter += 1)

  # Dispatching the event should invoke our closure via the dispatcher
  fireEvent(btn, "click")
  assert counter == 1, "callback should have been called once"

  fireEvent(btn, "click")
  assert counter == 2, "callback should have been called twice"

  # Register another callback on a different element
  let btn2 = r.createElement("button")
  var other = 0
  r.addEventListener(btn2, "click", proc() = other += 10)
  fireEvent(btn2, "click")
  assert other == 10, "second callback should work independently"

  # First callback still works
  fireEvent(btn, "click")
  assert counter == 3

  gpui_reset_tree()
  resetCallbacks()
  echo "test_renderer: callback registry unit test passed"

# ===========================================================================
# 7. Dynamic callback registry — scalability and ID monotonicity
# ===========================================================================

block dynamicCallbackScaleTest:
  gpui_reset_tree()
  resetCallbacks()

  var r: GpuiRenderer
  let parent = r.createElement("div")

  # Register 100+ callbacks and verify they all fire correctly
  type Counters = ref object
    data: array[150, int]
  let counters = Counters()
  var buttons: seq[GpuiElement]

  proc makeHandler(c: Counters; idx: int): proc() =
    result = proc() = c.data[idx] += 1

  for i in 0 ..< 150:
    let btn = r.createElement("button")
    r.appendChild(parent, btn)
    buttons.add btn
    r.addEventListener(btn, "click", makeHandler(counters, i))

  # Fire all events
  for i in 0 ..< 150:
    fireEvent(buttons[i], "click")

  # Verify all callbacks fired exactly once
  for i in 0 ..< 150:
    assert counters.data[i] == 1, "callback " & $i & " should have fired once, got: " & $counters.data[i]

  gpui_reset_tree()
  resetCallbacks()
  echo "test_renderer: dynamic callback scalability test passed (150 callbacks)"

block callbackIdMonotonicityTest:
  resetCallbacks()

  # Register some callbacks and verify IDs are monotonically increasing
  let id1 = registerCallback(proc() = discard)
  let id2 = registerCallback(proc() = discard)
  let id3 = registerCallback(proc() = discard)

  assert id1 < id2, "callback IDs should be monotonically increasing"
  assert id2 < id3, "callback IDs should be monotonically increasing"

  # Remove a callback and register a new one — ID should still increase
  removeCallback(id2)
  let id4 = registerCallback(proc() = discard)
  assert id4 > id3, "callback IDs should not be reused after removal"

  resetCallbacks()
  echo "test_renderer: callback ID monotonicity test passed"

# ===========================================================================
# 8. Tree operations (requires Rust shim linked)
# ===========================================================================

block treeOperations:
  gpui_reset_tree()
  resetCallbacks()

  var r: GpuiRenderer

  # Create elements
  let parent = r.createElement("div")
  assert parent != nil, "createElement should return non-nil"

  let child1 = r.createElement("span")
  let child2 = r.createElement("button")
  let textNode {.used.} = r.createTextNode("hello world")

  # Append children
  r.appendChild(parent, child1)
  r.appendChild(parent, child2)
  assert childCount(parent) == 2, "parent should have 2 children"

  # First child navigation
  let first = r.firstChild(parent)
  assert first != nil, "firstChild should return non-nil"

  # Next sibling navigation
  let second = r.nextSibling(first)
  assert second != nil, "nextSibling should return non-nil"

  # Parent navigation
  let p = r.parentNode(first)
  assert p != nil, "parentNode should return non-nil"

  # Text content
  r.setTextContent(child1, "hello")
  let tc = textContent(child1)
  assert tc == "hello", "textContent should be 'hello', got: " & tc

  # Insert before
  let child3 = r.createElement("li")
  r.insertBefore(parent, child3, child2)
  assert childCount(parent) == 3, "parent should have 3 children after insertBefore"

  # Remove child
  r.removeChild(parent, child3)
  assert childCount(parent) == 2, "parent should have 2 children after removeChild"

  # Attributes
  r.setAttribute(child1, "class", "active")
  let attr = getAttribute(child1, "class")
  assert attr == "active", "getAttribute should return 'active', got: " & attr

  r.removeAttribute(child1, "class")
  let removed = getAttribute(child1, "class")
  assert removed == "", "getAttribute after remove should return empty"

  # nth child
  let nth = nthChild(parent, 0)
  assert nth != nil, "nthChild(0) should return non-nil"

  gpui_reset_tree()
  echo "test_renderer: tree operations passed"

# ===========================================================================
# 9. Event dispatch (requires Rust shim linked)
# ===========================================================================

block eventDispatch:
  gpui_reset_tree()
  resetCallbacks()

  var r: GpuiRenderer
  let btn = r.createElement("button")

  var clicked = 0
  r.addEventListener(btn, "click", proc() = clicked += 1)

  # Fire the event via the shim
  fireEvent(btn, "click")
  assert clicked == 1, "click handler should have been called once, got: " & $clicked

  fireEvent(btn, "click")
  assert clicked == 2, "click handler should have been called twice"

  gpui_reset_tree()
  resetCallbacks()
  echo "test_renderer: event dispatch passed"

# ===========================================================================
# 10. Style application (requires Rust shim linked)
# ===========================================================================

block styleApplication:
  gpui_reset_tree()

  var r: GpuiRenderer
  let el = r.createElement("div")

  # Set various styles — these call through to the Rust shim
  r.setStyle(el, "width", "100px")
  r.setStyle(el, "height", "50px")
  r.setStyle(el, "background-color", "#ff0000")
  r.setStyle(el, "flex-direction", "row")
  r.setStyle(el, "align-items", "center")
  r.setStyle(el, "justify-content", "space-between")
  r.setStyle(el, "padding", "10px")
  r.setStyle(el, "gap", "8px")
  r.setStyle(el, "border-radius", "4px")
  r.setStyle(el, "cursor", "pointer")

  # If we got here without crashing, style mapping works
  gpui_reset_tree()
  echo "test_renderer: style application passed"

# ===========================================================================
# 11. Window compile-time type checks
# ===========================================================================

static:
  # Window creation
  assert compiles(createWindow("Test", 800.0, 600.0))

  # Window properties
  var w: GpuiWindow
  assert compiles(w.state)
  assert compiles(w.width)
  assert compiles(w.height)
  assert compiles(w.size)

  # Lifecycle
  assert compiles(w.show())
  assert compiles(w.close())
  assert compiles(w.destroy())

  # Callbacks
  assert compiles(w.onResize(proc(width, height: float) = discard))
  assert compiles(w.onFocus(proc(focused: bool) = discard))
  assert compiles(w.onClose(proc(): bool = true))

  # Repaint
  assert compiles(requestRepaint())
  assert compiles(repaintPending())

echo "test_renderer: window compile-time type checks passed"

# ===========================================================================
# 12. Window creation and lifecycle
# ===========================================================================

block windowCreation:
  resetWindows()
  gpui_reset_tree()

  let win = createWindow("Test Window", 1024.0, 768.0)
  assert win.id > 0, "window ID should be positive"
  assert win.state == wsCreated, "new window should be in Created state"
  assert win.width == 1024.0, "width should match creation parameter"
  assert win.height == 768.0, "height should match creation parameter"

  let (w, h) = win.size
  assert w == 1024.0
  assert h == 768.0

  win.destroy()
  echo "test_renderer: window creation passed"

block windowLifecycle:
  resetWindows()

  let win = createWindow("Lifecycle Test", 640.0, 480.0)
  assert win.state == wsCreated

  # Show
  assert win.show() == true, "show should succeed from Created"
  assert win.state == wsVisible

  # Cannot show again
  assert win.show() == false, "show should fail from Visible"
  assert win.state == wsVisible

  # Close
  assert win.close() == true, "close should succeed from Visible"
  assert win.state == wsClosed

  win.destroy()
  echo "test_renderer: window lifecycle passed"

# ===========================================================================
# 13. Window callbacks
# ===========================================================================

block resizeCallback:
  resetWindows()

  let win = createWindow("Resize Test", 800.0, 600.0)

  var resizedWidth = 0.0
  var resizedHeight = 0.0

  win.onResize proc(w, h: float) =
    resizedWidth = w
    resizedHeight = h

  # Simulate resize via the low-level notify function
  gpui_notify_resize(win.id, 1920.0, 1080.0)

  assert resizedWidth == 1920.0, "resize callback should receive new width"
  assert resizedHeight == 1080.0, "resize callback should receive new height"
  assert win.width == 1920.0, "window width should be updated"
  assert win.height == 1080.0, "window height should be updated"

  win.destroy()
  echo "test_renderer: resize callback passed"

block focusCallback:
  resetWindows()

  let win = createWindow("Focus Test", 800.0, 600.0)

  var lastFocusState = false

  win.onResize proc(w, h: float) = discard  # allocate slot first
  win.onFocus proc(focused: bool) =
    lastFocusState = focused

  # Simulate focus events
  gpui_notify_focus(win.id, 1)
  assert lastFocusState == true, "focus callback should receive true"

  gpui_notify_focus(win.id, 0)
  assert lastFocusState == false, "focus callback should receive false"

  win.destroy()
  echo "test_renderer: focus callback passed"

block closeCallbackAllow:
  resetWindows()

  let win = createWindow("Close Allow Test", 800.0, 600.0)
  discard win.show()

  win.onResize proc(w, h: float) = discard  # allocate slot
  win.onClose proc(): bool = true  # allow close

  assert win.close() == true, "close should be allowed"
  assert win.state == wsClosed

  win.destroy()
  echo "test_renderer: close callback (allow) passed"

block closeCallbackDeny:
  resetWindows()

  let win = createWindow("Close Deny Test", 800.0, 600.0)
  discard win.show()

  win.onResize proc(w, h: float) = discard
  win.onClose proc(): bool = false  # deny close

  assert win.close() == false, "close should be denied"
  assert win.state == wsVisible, "window should remain Visible after denied close"

  win.destroy()
  echo "test_renderer: close callback (deny) passed"

# ===========================================================================
# 14. Repaint integration
# ===========================================================================

block repaintIntegration:
  resetWindows()
  gpui_reset_tree()
  discard repaintPending()  # clear

  # Create elements via raw bindings
  let parent = gpui_create_element("div".cstring)
  let child = gpui_create_element("text".cstring)
  discard repaintPending()  # clear any residual

  # appendChild should trigger repaint
  gpui_append_child(parent, child)
  assert repaintPending() == true, "appendChild should request repaint"

  # setAttribute should trigger repaint
  gpui_set_attribute(parent, "width".cstring, "100".cstring)
  assert repaintPending() == true, "setAttribute should request repaint"

  # setStyle should trigger repaint
  gpui_set_style(parent, "bg".cstring, "red".cstring)
  assert repaintPending() == true, "setStyle should request repaint"

  # setTextContent should trigger repaint
  gpui_set_text_content(child, "hello".cstring)
  assert repaintPending() == true, "setTextContent should request repaint"

  # removeChild should trigger repaint
  gpui_remove_child(parent, child)
  assert repaintPending() == true, "removeChild should request repaint"

  # insertBefore should trigger repaint
  gpui_append_child(parent, child)
  discard repaintPending()  # clear
  let child2 = gpui_create_element("text".cstring)
  discard repaintPending()
  gpui_insert_before(parent, child2, child)
  assert repaintPending() == true, "insertBefore should request repaint"

  # No more pending
  assert repaintPending() == false, "no repaint should be pending"

  echo "test_renderer: repaint integration passed"

block manualRepaint:
  resetWindows()
  discard repaintPending()  # clear

  requestRepaint()
  assert repaintPending() == true, "manual requestRepaint should set flag"
  assert repaintPending() == false, "flag should be cleared after take"

  echo "test_renderer: manual repaint passed"

# ===========================================================================
# 15. Multiple windows
# ===========================================================================

block multipleWindows:
  resetWindows()

  let win1 = createWindow("Window 1", 800.0, 600.0)
  let win2 = createWindow("Window 2", 1024.0, 768.0)

  assert win1.id != win2.id, "windows should have different IDs"
  assert win1.width == 800.0
  assert win2.width == 1024.0

  discard win1.show()
  assert win1.state == wsVisible
  assert win2.state == wsCreated

  discard win2.show()
  assert win2.state == wsVisible

  discard win1.close()
  assert win1.state == wsClosed
  assert win2.state == wsVisible

  win1.destroy()
  win2.destroy()
  echo "test_renderer: multiple windows passed"

# ===========================================================================
# 16. Window not found handling
# ===========================================================================

block windowNotFound:
  resetWindows()

  let fake = GpuiWindow(id: 999)
  assert fake.state == wsNotFound, "nonexistent window should be wsNotFound"
  assert fake.width == 0.0
  assert fake.height == 0.0
  assert fake.show() == false
  assert fake.close() == false
  # destroy should not crash
  fake.destroy()

  echo "test_renderer: window not found handling passed"

echo "test_renderer: all tests passed"
