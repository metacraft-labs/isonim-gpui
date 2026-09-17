## NH-M1 verification: test_render_native_gpui_reactive
##
## Claim under test (from
## `codetracer-specs/Front-Ends/IsoNim/Hot-Module-Reload-Native.milestones.org`,
## NH-M1): the GPUI mount path runs a render effect, so mutating a signal the
## root proc reads rebuilds and re-inserts the root — "same against GPUI's
## element tree".
##
## MOCK POLICY: no mocks. Every element here is created through
## `GpuiRenderer`, i.e. through the real `gpui_*` FFI into the Rust shim
## (`libgpui_nim_shim`); the assertions read the tree back with
## `gpui_child_count` / `gpui_get_text_content` through `childCount` and
## `textContent`, so what is measured is the SHIM's element graph, not a Nim
## side-copy. The reactive core is the real one from `isonim/core`.
##
## PREREQUISITE — LOUD, NOT SKIPPED. Every binding in `bindings.nim` is
## `{.dynlib: libgpui_nim_shim.so.}`. `bindings.nim` bakes in the absolute
## `rust/target/debug/` path when the cdylib exists AT COMPILE TIME and
## otherwise emits the bare soname to be resolved from the loader path; either
## way, a binary built and run without the shim dies before the first test
## body with "could not load: libgpui_nim_shim.so" — a hard non-zero exit, and
## the correct failure. There is deliberately NO probe-and-skip arm here: a
## suite that reported "N passed" without the shim would be asserting nothing
## about GPUI. Build it with `just rust-build` first.
##
## DISCRIMINATION. The reactive cases are paired with controls that build and
## attach the identical tree WITHOUT the seam (and with the seam untracked via
## `staticNativeRoot`) and assert the tree does NOT change on the same signal
## write.

import std/strutils
import unittest

import isonim_gpui/renderer
import isonim_gpui/bindings
import isonim_gpui/reactive_root

import isonim/core/signals

proc buildLabelTree(r: GpuiRenderer; text: string): GpuiElement =
  ## A fresh root every call — the root-swap path NH-M2's proxy will drive.
  let root = r.createElement("div")
  r.setAttribute(root, "class", "app-root")
  let line = r.createElement("span")
  r.setTextContent(line, text)
  r.appendChild(root, line)
  root

suite "NH-M1: renderGpui reactive root (isonim-gpui)":

  test "test_render_native_gpui_reactive":
    gpui_reset_tree()
    resetCallbacks()
    let r = GpuiRenderer()
    let host = r.createElement("div")
    let label = createSignal("ALPHA")

    let handle = renderGpui(r, host,
      NativeRootAccessor[GpuiElement](proc(): GpuiElement =
        buildLabelTree(r, label.val)))

    check handle.renders == 1
    check handle.rootSwaps == 1
    check childCount(host) == 1
    check contains(textContent(host), "ALPHA")
    # Proof the Rust shim is really in the loop rather than a Nim side-copy:
    # `gpui_tree_node_count` is the SHIM's own registry size, and it grew by
    # the three elements the accessor created plus the host.
    let nodesAfterMount = gpui_tree_node_count()
    check nodesAfterMount >= 3'u64

    # The claim: a signal write in the root proc fires the render effect and
    # the shim's element graph carries the new tree.
    label.val = "BETA"

    check handle.renders == 2
    check handle.rootSwaps == 2
    # Replaced, not appended: the old root was removed from the shim tree.
    check childCount(host) == 1
    check contains(textContent(host), "BETA")
    check not contains(textContent(host), "ALPHA")
    # The rebuild created elements in the shim, not in Nim.
    check gpui_tree_node_count() > nodesAfterMount

    label.val = "GAMMA"
    check handle.renders == 3
    check childCount(host) == 1
    check contains(textContent(host), "GAMMA")

    handle.dispose()

  test "test_control_gpui_imperative_mount_does_not_rebuild_on_signal_write":
    # DISCRIMINATION CONTROL — the pre-NH-M1 call shape (build once, attach by
    # hand). If the shim tree changed here the case above would prove nothing.
    gpui_reset_tree()
    resetCallbacks()
    let r = GpuiRenderer()
    let host = r.createElement("div")
    let label = createSignal("ALPHA")

    r.appendChild(host, buildLabelTree(r, label.val))
    check contains(textContent(host), "ALPHA")

    label.val = "BETA"

    check childCount(host) == 1
    check contains(textContent(host), "ALPHA")
    check not contains(textContent(host), "BETA")

  test "test_control_gpui_static_native_root_keeps_build_once_semantics":
    # The "no behaviour change for non-HMR callers" deliverable, measured
    # against the shim: seam present, accessor untracked (web `render()`'s
    # shape). Must be indistinguishable from the imperative control.
    gpui_reset_tree()
    resetCallbacks()
    let r = GpuiRenderer()
    let host = r.createElement("div")
    let label = createSignal("ALPHA")

    let handle = renderGpui(r, host,
      staticNativeRoot(proc(): GpuiElement = buildLabelTree(r, label.val)))
    check handle.renders == 1
    check contains(textContent(host), "ALPHA")

    label.val = "BETA"

    check handle.renders == 1
    check childCount(host) == 1
    check contains(textContent(host), "ALPHA")
    check not contains(textContent(host), "BETA")
    handle.dispose()

  test "test_render_native_gpui_mount_callback_publishes_the_current_root":
    # The overload the EX-M14 GPUI launcher uses: GPUI has no ambient root
    # slot, so the launcher publishes the current root to its frame source
    # through the `mount` callback. It must run on every effect pass.
    gpui_reset_tree()
    resetCallbacks()
    let r = GpuiRenderer()
    let label = createSignal("ALPHA")
    var published: GpuiElement = nil
    var publishes = 0

    let handle = renderGpui(
      NativeRootAccessor[GpuiElement](proc(): GpuiElement =
        buildLabelTree(r, label.val)),
      NativeRootMount[GpuiElement](proc(node: GpuiElement) =
        inc publishes
        published = node))

    check publishes == 1
    check published != nil
    check contains(textContent(published), "ALPHA")

    label.val = "BETA"

    check publishes == 2
    check handle.renders == 2
    check contains(textContent(published), "BETA")
    handle.dispose()

  test "test_render_native_gpui_dispose_stops_the_render_effect":
    gpui_reset_tree()
    resetCallbacks()
    let r = GpuiRenderer()
    let host = r.createElement("div")
    let label = createSignal("ALPHA")

    let handle = renderGpui(r, host,
      NativeRootAccessor[GpuiElement](proc(): GpuiElement =
        buildLabelTree(r, label.val)))
    label.val = "BETA"
    check handle.renders == 2

    handle.dispose()
    label.val = "GAMMA"

    check handle.renders == 2
    check contains(textContent(host), "BETA")
    check not contains(textContent(host), "GAMMA")
    check isDisposed(handle)
