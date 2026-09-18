## NH-M3 — ``test_gpui_reconciler_preserves_identity``
##
## Stub-driven: a real reload runs through the HCR agent seam, the entry
## call re-registers the ui slots, and the reconciler folds the rebuilt
## tree into the live one. The assertion is on the shadow-tree NODE
## IDENTITY that survives.
##
## MOCK POLICY (workspace rule: every mock justified in the header).
## Exactly ONE double — ``isonim/tests/helpers/hcr_stub.nim``, the
## Reprobuild HCR agent — justified at length in that file: the shipped
## ``librepro_hcr_agent`` exports the ten ``rb_hcr_*`` symbols with
## baseline bodies that never fire a callback, and live dispatch is
## Reprobuild HLX-M8, which is ``planned``, so the real library would
## exercise the reload lifecycle zero times.
##
## Everything else is the production object: the REAL Rust shim (every
## ``gpui_*`` call below crosses the FFI into ``libgpui_nim_shim``), the
## real slot registry and memos from ``isonim/native/hmr``, and the real
## engine from ``isonim/native/reconciler`` driven through this repo's
## instance.
##
## NO SKIP ARMS. The shim is a hard requirement — every binding is
## ``dynlib``, so without ``just rust-build`` this binary does not start
## at all. That failure is deliberate and is not skipped: a suite that
## went green without the shim would be asserting nothing about GPUI.
##
## ## Why identity is asserted with ``sameNode`` and not ``==``
##
## ``GpuiElement`` is an opaque handle, and **every child/parent accessor
## in the shim mints a fresh ``Box``** (``node_id_to_handle``). Two
## handles to the same shadow node are therefore different pointers, and
## a gate written with ``==`` would report "the node was rebuilt" for a
## node that was not touched — a false red that the obvious repair
## (weaken the assertion) would turn into a permanent false green.
## ``sameNode`` compares ``gpui_node_id``, which is the identity.
##
## ## Discrimination
##
## Each "preserves X" case is paired with a measured control running the
## SAME predicate over the SAME trees with an always-replace reconciler
## (``identityKey`` never repeats). One predicate, two subjects
## (`Verification-Harness-Traps` §30), and the control is asserted to
## FAIL the preservation it controls for, so it is not a self-comparison
## wearing a negation (§7b).

when not defined(isonimHmr):
  {.error: "test_gpui_reconciler_identity must be compiled with " &
      "-d:isonimHmr. Without the flag isonim/native/hmr has no registry " &
      "and no slots, so the 'reload' this file drives would not happen " &
      "and every assertion would be about a tree that was never rebuilt.".}

import std/strutils
import unittest

import isonim_gpui/bindings
import isonim_gpui/renderer
import isonim_gpui/reconciler

import isonim/native/hmr
import isonim/native/reconciler

import ../../isonim/tests/helpers/hcr_stub

const
  slotRows = "gpui_recon_demo.nim:21:2"

var r: GpuiRenderer
var rowsVersion = 1

proc keyed(tag, key: string; text: string = ""): GpuiElement =
  result = r.createElement(tag)
  r.setAttribute(result, IsonimKeyAttr, key)
  if text.len > 0:
    r.setTextContent(result, text)

proc makeRows(version: int): UiSlotFactory =
  ## Three keyed rows; only the middle one carries the version. So a
  ## hash change edits exactly one leaf and leaves its two siblings
  ## byte-identical, which is what makes "the unchanged parts of a
  ## CHANGED block keep their identity" measurable rather than claimed.
  uiSlotFactory(proc(): GpuiElement =
    let rows = keyed("div", "rows")
    r.appendChild(rows, keyed("div", "rowA", "alpha"))
    r.appendChild(rows, keyed("div", "rowB", "beta" & $version))
    r.appendChild(rows, keyed("div", "rowC", "gamma"))
    rows)

proc demoEntry() =
  hmrRegisterFactory(slotRows, "rows" & $rowsVersion, makeRows(rowsVersion))

proc demoRoot(): GpuiElement =
  let root = keyed("div", "root")
  r.appendChild(root, hmrInvokeSlot[GpuiElement](slotRows))
  root

proc detachedRootAt(version: int): GpuiElement =
  ## A complete tree built WITHOUT going through the slot memo, so it
  ## shares no node with any other tree. See the "appendChild re-parents"
  ## case below for why that matters on a shim-backed renderer.
  let root = keyed("div", "root")
  r.appendChild(root, unboxUiNode[GpuiElement](makeRows(version)()))
  root

proc childByKey(n: GpuiElement; key: string): GpuiElement =
  if n == nil: return nil
  for i in 0 ..< childCount(n):
    let c = nthChild(n, i)
    if getAttribute(c, IsonimKeyAttr) == key: return c
  nil

proc deepChild(n: GpuiElement; path: varargs[string]): GpuiElement =
  result = n
  for key in path:
    if result == nil: return nil
    result = result.childByKey(key)

var alwaysReplaceCounter = 0

proc newAlwaysReplaceReconciler(): RendererReconciler[GpuiElement] =
  result = newGpuiReconciler()
  result.nodes.identityKey = proc(n: GpuiElement): NodeIdentity =
    inc alwaysReplaceCounter
    "unmatchable-" & $alwaysReplaceCounter

type ReloadOutcome = object
  rootSurvived: bool
  unchangedRowSurvived: bool
  changedRowSurvived: bool
  changedRowText: string
  stats: ReconcileStats

proc runReload(rec: RendererReconciler[GpuiElement];
               bumpRowsHash: bool): ReloadOutcome =
  ## Mount, drive one reload through the stub agent, reconcile, report.
  ## Both the real reconciler and the control go through this, so
  ## neither is graded on its own yardstick.
  gpui_reset_tree()
  resetCallbacks()
  r = GpuiRenderer()
  rowsVersion = 1
  let stub = installHcrStub()
  let root = newHmrRoot(demoEntry)
  root.start()

  let liveRoot = demoRoot()
  let oldRootId = nodeId(liveRoot)
  let oldRowAId = nodeId(liveRoot.deepChild("rows", "rowA"))
  let oldRowBId = nodeId(liveRoot.deepChild("rows", "rowB"))
  doAssert oldRootId != 0 and oldRowAId != 0 and oldRowBId != 0,
    "fixture: the pre-reload tree does not have the shape this test assumes"

  # The version bump happens inside `applyCodeSwap`, i.e. at Phase G —
  # the only point at which the "new bodies" become reachable. Bumping
  # before `rbHcrApplyReload` would make the new body visible to Phase E
  # and the gate would pass under either phase ordering.
  stub.queuePatch(HcrStubPatch(
    changedFiles: @["gpui_recon_demo.nim"],
    changedTypes: @[],
    applyCodeSwap: proc() =
      if bumpRowsHash: rowsVersion = 2))
  doAssert rbHcrWantsReload()
  rbHcrApplyReload()
  doAssert root.appliedReloads == 1, "the stub-driven reload did not apply"
  doAssert root.failedReloads == 0

  let rebuiltRoot = demoRoot()
  var stats = ReconcileStats()
  let survivingRoot = rec.reconcile(liveRoot, rebuiltRoot, stats)

  let survivingRowB = survivingRoot.deepChild("rows", "rowB")
  ReloadOutcome(
    rootSurvived: nodeId(survivingRoot) == oldRootId,
    unchangedRowSurvived:
      nodeId(survivingRoot.deepChild("rows", "rowA")) == oldRowAId,
    changedRowSurvived: nodeId(survivingRowB) == oldRowBId,
    changedRowText:
      (if survivingRowB == nil: "" else: textContent(survivingRowB)),
    stats: stats)

suite "NH-M3: GPUI reconciler preserves identity":

  test "test_gpui_reconciler_preserves_identity":
    let outcome = newGpuiReconciler().runReload(bumpRowsHash = true)
    check outcome.rootSurvived
    check outcome.unchangedRowSurvived
    # The edited row is the SAME node with new content — a changed body
    # means new props, not a new element. This is what keeps GPUI/Freya
    # computed-style caches alive on everything around it.
    check outcome.changedRowSurvived
    check outcome.changedRowText.contains("beta2")
    check outcome.stats.propUpdates == 1
    check outcome.stats.placed == 0
    check outcome.stats.moved == 0
    check outcome.stats.removed == 0

  test "CONTROL: with no reconciler, the same reload loses every reference":
    let outcome = newAlwaysReplaceReconciler().runReload(bumpRowsHash = true)
    check not outcome.rootSurvived
    check not outcome.unchangedRowSurvived
    check not outcome.changedRowSurvived
    check outcome.stats.matched == 0

  test "an IDENTICAL rebuild touches the renderer zero times":
    # The "when a slot's hash MATCHES, the reconciler does nothing" half
    # of the milestone's claim, measured as a census of renderer calls
    # because the drawn frame is identical either way.
    #
    # Built through `detachedRootAt` rather than by driving a no-op
    # reload, and the reason is the finding recorded in the next case:
    # a rebuild that goes through the slot memo hands the NEW root the
    # SAME node the old root is holding, and `gpui_append_child`
    # re-parents it — so the "old" tree would be dismantled before the
    # reconciler ever saw it.
    gpui_reset_tree()
    r = GpuiRenderer()
    let before = detachedRootAt(1)
    let oldRowA = nodeId(before.deepChild("rows", "rowA"))
    let oldRowC = nodeId(before.deepChild("rows", "rowC"))
    let after = detachedRootAt(1)
    var stats = ReconcileStats()
    let surviving = newGpuiReconciler().reconcile(before, after, stats)
    check nodeId(surviving) == nodeId(before)
    check nodeId(surviving.deepChild("rows", "rowA")) == oldRowA
    check nodeId(surviving.deepChild("rows", "rowC")) == oldRowC
    check stats.touched == 0
    # Non-vacuity: the pass must have walked a real tree rather than
    # returning early. root + rows + three rows, at least.
    check stats.matched >= 5

  test "RECORDED: gpui_append_child RE-PARENTS, which is why a memo-built root cannot be reconciled in place":
    # Measured 2026-09-18 and recorded here because it is a property of
    # the shim, not of this test, and it constrains how NH-M4 may wire
    # the reconciler into `mountUiHot` on the shim-backed renderers.
    #
    # The shadow tree is global and `append_child` MOVES a node that
    # already has a parent. So the natural integration — "re-run the
    # entry, build the new root, then reconcile it against the live one"
    # — destroys the live root as a side effect of building the new one:
    # every slot whose hash did NOT change is served from its memo, is
    # the same node, and is stolen from the old parent on append.
    #
    # NH-M4 has to either build the candidate tree detached (a second
    # shadow tree, or nodes parked under a scratch root) or drive the
    # reconciler from the slot boundary instead of the root. The gate
    # above uses `detachedRootAt` for exactly this reason.
    #
    # If this ever goes red, `append_child` has stopped re-parenting and
    # the constraint is lifted — record that, do not delete this case.
    gpui_reset_tree()
    r = GpuiRenderer()
    let p1 = r.createElement("div")
    let p2 = r.createElement("div")
    let shared = r.createElement("span")
    r.appendChild(p1, shared)
    check childCount(p1) == 1
    r.appendChild(p2, shared)
    check childCount(p1) == 0      # …the first parent lost it
    check childCount(p2) == 1

  test "handles to one node are different POINTERS — the premise of sameNode":
    # Guards the reason every assertion above compares `nodeId` rather
    # than `==`. If the shim ever started returning cached handles this
    # would go red, and the right response is to record that the premise
    # changed — not to switch the gates back to pointer equality, which
    # would then be silently correct for the wrong reason.
    gpui_reset_tree()
    r = GpuiRenderer()
    let parent = keyed("div", "p")
    let child = keyed("div", "c")
    r.appendChild(parent, child)
    let viaAccessor = nthChild(parent, 0)
    check viaAccessor != child                  # different pointers …
    check sameNode(viaAccessor, child)          # … same node
    check nodeId(viaAccessor) == nodeId(child)

  test "the GPUI instance satisfies the whole contract":
    newGpuiReconciler().validate()
