## Performance benchmarks for the GPUI shim.
##
## Measures element creation, tree mutation, and text content update throughput
## to establish baseline performance metrics for the GPUI renderer backend.
##
## Build & run:
##   LD_LIBRARY_PATH=rust/target/debug nim c -r -d:release \
##     --path:../isonim/src --nimcache:nimcache/test_performance \
##     tests/test_performance.nim

import std/[times, strutils, strformat]
import isonim_gpui/renderer
import isonim_gpui/bindings

# ============================================================================
# Benchmark harness
# ============================================================================

type
  BenchResult = object
    name: string
    count: int
    elapsed: float   # seconds
    opsPerSec: float

proc bench(name: string; count: int; body: proc()): BenchResult =
  let start = cpuTime()
  body()
  let elapsed = cpuTime() - start
  let opsPerSec = if elapsed > 0: count.float / elapsed else: 0.0
  result = BenchResult(
    name: name,
    count: count,
    elapsed: elapsed,
    opsPerSec: opsPerSec,
  )

proc report(r: BenchResult) =
  let opsStr = formatFloat(r.opsPerSec, ffDecimal, 0)
  let timeStr = formatFloat(r.elapsed * 1000, ffDecimal, 2)
  echo &"  {r.name}: {r.count} ops in {timeStr}ms ({opsStr} ops/sec)"

# ============================================================================
# Benchmarks
# ============================================================================

const N = 10_000
const SMALL_N = 1_000

proc benchElementCreation(): BenchResult =
  gpui_reset_tree()
  bench("Element creation", N, proc() =
    let r = GpuiRenderer()
    for i in 0 ..< N:
      discard r.createElement("div")
  )

proc benchTextNodeCreation(): BenchResult =
  gpui_reset_tree()
  bench("Text node creation", N, proc() =
    let r = GpuiRenderer()
    for i in 0 ..< N:
      discard r.createTextNode("hello world " & $i)
  )

proc benchAppendChild(): BenchResult =
  gpui_reset_tree()
  let r = GpuiRenderer()
  let parent = r.createElement("div")
  # Pre-create children
  var children: seq[GpuiElement]
  for i in 0 ..< N:
    children.add r.createElement("span")
  bench("appendChild", N, proc() =
    for c in children:
      r.appendChild(parent, c)
  )

proc benchRemoveChild(): BenchResult =
  gpui_reset_tree()
  let r = GpuiRenderer()
  let parent = r.createElement("div")
  var children: seq[GpuiElement]
  for i in 0 ..< N:
    let c = r.createElement("span")
    r.appendChild(parent, c)
    children.add c
  bench("removeChild", N, proc() =
    for c in children:
      r.removeChild(parent, c)
  )

proc benchSetTextContent(): BenchResult =
  gpui_reset_tree()
  let r = GpuiRenderer()
  let node = r.createTextNode("initial")
  bench("setTextContent", N, proc() =
    for i in 0 ..< N:
      r.setTextContent(node, "text update " & $i)
  )

proc benchSetAttribute(): BenchResult =
  gpui_reset_tree()
  let r = GpuiRenderer()
  let node = r.createElement("div")
  bench("setAttribute", N, proc() =
    for i in 0 ..< N:
      r.setAttribute(node, "class", "class-" & $i)
  )

proc benchSetStyle(): BenchResult =
  gpui_reset_tree()
  let r = GpuiRenderer()
  let node = r.createElement("div")
  bench("setStyle", N, proc() =
    for i in 0 ..< N:
      r.setStyle(node, "width", $(i mod 1000) & "px")
  )

proc benchTreeBuild(): BenchResult =
  ## Build a realistic tree: 100 rows, each with 5 children
  gpui_reset_tree()
  let rows = SMALL_N
  let cols = 5
  bench("Tree build (" & $rows & "x" & $cols & ")", rows * cols, proc() =
    let r = GpuiRenderer()
    let root = r.createElement("div")
    for i in 0 ..< rows:
      let row = r.createElement("div")
      for j in 0 ..< cols:
        let cell = r.createElement("span")
        r.setTextContent(cell, "R" & $i & "C" & $j)
        r.appendChild(row, cell)
      r.appendChild(root, row)
  )

proc benchGetTextContent(): BenchResult =
  gpui_reset_tree()
  let r = GpuiRenderer()
  let parent = r.createElement("div")
  for i in 0 ..< 10:
    let child = r.createElement("span")
    r.setTextContent(child, "child " & $i)
    r.appendChild(parent, child)
  bench("getTextContent (10 children)", N, proc() =
    for i in 0 ..< N:
      discard textContent(parent)
  )

proc benchChildCount(): BenchResult =
  gpui_reset_tree()
  let r = GpuiRenderer()
  let parent = r.createElement("div")
  for i in 0 ..< 100:
    r.appendChild(parent, r.createElement("span"))
  bench("childCount (100 children)", N, proc() =
    for i in 0 ..< N:
      discard childCount(parent)
  )

proc benchNthChild(): BenchResult =
  gpui_reset_tree()
  let r = GpuiRenderer()
  let parent = r.createElement("div")
  for i in 0 ..< 100:
    r.appendChild(parent, r.createElement("span"))
  bench("nthChild (100 children)", N, proc() =
    for i in 0 ..< N:
      discard nthChild(parent, i mod 100)
  )

proc benchEventDispatch(): BenchResult =
  gpui_reset_tree()
  resetCallbacks()
  let r = GpuiRenderer()
  let btn = r.createElement("button")
  var counter = 0
  r.addEventListener(btn, "click", proc() = inc counter)
  bench("Event dispatch", N, proc() =
    for i in 0 ..< N:
      fireEvent(btn, "click")
  )

# ============================================================================
# Main
# ============================================================================

when isMainModule:
  echo "=== GPUI Shim Performance Benchmarks ==="
  echo &"  N = {N}, SMALL_N = {SMALL_N}"
  echo ""

  echo "--- Element Creation ---"
  benchElementCreation().report()
  benchTextNodeCreation().report()

  echo ""
  echo "--- Tree Mutation ---"
  benchAppendChild().report()
  benchRemoveChild().report()
  benchTreeBuild().report()

  echo ""
  echo "--- Content Updates ---"
  benchSetTextContent().report()
  benchSetAttribute().report()
  benchSetStyle().report()

  echo ""
  echo "--- Tree Inspection ---"
  benchGetTextContent().report()
  benchChildCount().report()
  benchNthChild().report()

  echo ""
  echo "--- Events ---"
  benchEventDispatch().report()

  echo ""
  echo "All benchmarks completed."
