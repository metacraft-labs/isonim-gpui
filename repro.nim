## Reprobuild project file for isonim-gpui.
##
## **Typed-Cross-Project-Deps rollout — an IsoNim renderer CONSUMER with a
## native Rust cdylib.** isonim-gpui is the GPUI (Zed's GPU-accelerated UI)
## RendererBackend for IsoNim. Its Nim surface (``src/isonim_gpui/{bindings,
## renderer,window}.nim``) is a thin FFI wrapper over a Rust cdylib
## (``rust/gpui-nim-shim``) that maintains an in-memory shadow element tree
## + render plan; the Nim tests exercise that FFI. So this recipe has TWO
## kinds of inputs the leaf recipes (nim-pty / nim-libvterm) don't:
##
## 1. **A native cargo cdylib.** ``rust/gpui-nim-shim`` builds a
##    ``cdylib`` named ``libgpui_nim_shim.so`` (``.dylib`` / ``.dll`` on
##    macOS / Windows) under ``rust/target/debug/`` via the DEFAULT
##    (feature-less, "stub") cargo profile — the shadow-tree + extern-"C"
##    API without the ``gpui-backend`` / ``gpui-headless`` features (which
##    pull in the ``zed-industries/zed`` git checkout + a GPU/display and
##    are only needed for the on-screen / headless-RGBA render paths, NOT
##    for the FFI corpus here). ``bindings.nim`` computes an ABSOLUTE
##    ``dynlib`` path at Nim-compile time (``currentSourcePath()... /
##    rust/target/debug/libgpui_nim_shim.so`` guarded by ``fileExists``),
##    so as long as the ``.so`` exists at that path when a test COMPILES,
##    the produced binary ``dlopen``s it by absolute path at run time — no
##    ``LD_LIBRARY_PATH`` needed. Every test BUILD edge therefore runs
##    ``after`` the cargo build and declares the ``.so`` as an
##    ``extraInput``; every EXECUTE edge lists it via ``requiredBinaries``
##    so the engine (a) builds the shim first and (b) re-runs the test when
##    the shim content changes. This mirrors ``just rust-build`` +
##    ``LD_LIBRARY_PATH=rust/target/debug`` in the ``Justfile``.
##
## 2. **The ``isonim`` sibling library (+ its ``nim-everywhere`` dep).**
##    The cross-renderer / render-plan / structural tests
##    (``test_cross_renderer`` / ``test_render_integration`` /
##    ``test_structural_comparison``) ``import isonim/...`` — the reactive
##    core (``core/{signals,computation,owner}``), the mock DOM
##    (``testing/mock_dom``), and the demo terminal renderer
##    (``renderers/terminal_demo``). ``isonim/core/platform`` re-exports
##    ``nim_everywhere/platform``, so those tests transitively need the
##    ``nim-everywhere`` sibling too. Both are in the rollout AVAILABLE set
##    (each ships a landed ``repro.nim`` — ``library isonim`` /
##    ``library nim_everywhere``, both exporting ``src``), so this is
##    proper SC-11 develop-mode from-source consumption: naming them in
##    ``uses:`` makes reprobuild build each from source and thread its
##    ``src/`` root onto THIS repo's ``nim c --path:`` via the SC-11
##    ``nimPathDirs`` aux channel (Cross-Repo-Source-Consumption.md §4.2a)
##    — replacing the ``Justfile``'s hardcoded ``--path:../isonim/src``.
##    NOT a SKIP, NOT a hardcoded path.
##
## A Mode 1 / Mode 3 hybrid (per
## ``reprobuild-specs/Three-Mode-Convention-System.md``) modelled on the
## canonical Nim-consumer recipes ``nim-agents/repro.nim`` (multi-sibling
## consumer) + the ``codetracer-js-recorder/repro.nim`` cargo-cdylib build
## shape, plus the ``nim-libvterm`` / ``nim-pty`` two-edge test template.
##
## **Compile flags.** isonim-gpui ships a ``nim.cfg`` with
## ``--path:"src" --path:"../isonim/src"``. The ``src`` root is supplied by
## the ``library isonim_gpui`` export + the per-edge ``paths = @["src"]``;
## the ``../isonim/src`` literal is REPLACED by the SC-11 ``uses: "isonim"``
## edge (reprobuild threads isonim's ``src`` on ``--path`` from source). No
## ``config.nims`` and no memory-manager pin in the repo — the ``Justfile``
## ``test`` recipe is a bare ``nim c -r`` (Nim's per-file default mm), so
## the edges pass no ``mm:`` / ``-d:`` (except ``test_performance``, which
## the repo's own ``test-perf`` recipe builds ``-d:release``).
##
## **Per-test platform gating.** No test file in ``tests/`` carries a
## ``{.error.}`` module guard or an OS-only ``import`` that would make it
## non-runnable on this Linux host. All eight compile + run to exit 0 here.
## The single OS-ish conditional in the corpus is ``test_gui.nim``'s
## ``when defined(gpuiBackend):`` suite (the "GPUI Backend Compile Check"
## that drives real window state) — that arm is ADDITIVE and only compiled
## when ``-d:gpuiBackend`` is passed (which also requires the cargo shim be
## built ``--features gpui-backend`` + a GPU/display, the repo's
## ``test-gui-x11`` / ``test-gui-wayland`` Xvfb/Sway path). The DEFAULT
## build of ``test_gui.nim`` (no ``-d:gpuiBackend``, stub shim) runs the
## render-plan + launch-integration suites headless with no display — which
## is exactly what this edge compiles and runs. So there is no host-
## exclusive gate: the whole corpus is in the Linux graph, and the display-
## only ``gpuiBackend`` suite is simply not compiled here (matching the
## repo's own default ``just test`` / ``just test-all`` set — the GPU
## window path is a separate ``test-gui-*`` recipe, not part of ``test-all``).
##
##   * ``test_basic`` / ``test_bindings`` — pure ``static:`` compile-time
##     conformance checks over ``isonim_gpui/{renderer,bindings}``. Leaf
##     (no isonim). Each still produces a runnable binary that echoes a
##     pass line (exit 0). No FFI CALLED at run time, but they still link
##     the shim ``dynlib`` const, so the ``.so`` is required at compile.
##   * ``test_renderer`` — tag/style/attribute mapping + tree ops + window
##     lifecycle. Leaf (no isonim); CALLS the shim FFI at run time.
##   * ``test_performance`` — micro-benchmarks over the shim FFI; the
##     repo's ``test-perf`` recipe builds it ``-d:release``, reproduced via
##     ``defines = @["release"]``. Leaf; CALLS the FFI at run time.
##   * ``test_gui`` — render-plan smoke + launch-integration suites (stub
##     shim, headless). Leaf (imports only ``isonim_gpui`` + std); CALLS
##     the FFI at run time.
##   * ``test_cross_renderer`` — cross-renderer equivalence vs isonim's
##     MockRenderer + demo TerminalRenderer. CONSUMER: ``import
##     isonim/{testing/mock_dom,renderers/terminal_demo,core/*}`` →
##     ``uses: "isonim"`` + transitively ``uses: "nim-everywhere"``.
##   * ``test_render_integration`` / ``test_structural_comparison`` —
##     render-plan integration + structural comparison driven off isonim's
##     reactive core (``core/{signals,computation,owner}``). CONSUMER:
##     ``uses: "isonim"`` + ``uses: "nim-everywhere"``.
##
## **Tool provisioning.** ``defaultToolProvisioning "path"`` matches the
## canonical recipes: the nix dev shell puts ``nim`` + ``gcc`` + ``cargo``
## + ``rustc`` on ``PATH``. Without it ``repro build`` refuses to run with
## "typed tool provisioning is required for uses declarations".

import std/os
import repro_project_dsl

# ``ct_test_nim_unittest`` supplies ``buildNimUnittest.build(...)`` (the
# per-test compile BUILD edge) and the ``edge.testBinary.run(...)`` UFCS
# dispatch (the EXECUTE edge). ``repro_dsl_stdlib/packages/cargo`` supplies
# the ``cargo.build(...)`` typed tool for the native shim. Both re-export
# ``repro_project_dsl`` so import order is unimportant. Like the
# ``nim-libvterm`` / ``nim-pty`` leaf recipes this file does NOT import
# ``ct_test_runner_install`` (engine-coupled, reprobuild-internal): the
# execute edges route through the engine's default direct-binary runner
# (run the binary, key on exit status) — exactly the exit-0 verification
# this corpus needs (Nim ``unittest`` exits non-zero on failure).
import ct_test_nim_unittest

type
  GpuiTestSpec = object
    ## One entry per runnable test file. ``source`` is the repo-relative
    ## ``.nim`` path; ``binary`` is the ``build/test-bin/<stem>`` output.
    ## ``consumesIsonim`` marks the CONSUMER tests that ``import isonim/…``
    ## (they need the SC-11 ``isonim`` + ``nim-everywhere`` sibling ``src``
    ## roots on ``--path``; the engine threads those off the ``uses:``
    ## edges, so no per-edge flag is required — the field is documentation
    ## + a future hook). ``releaseBuild`` marks ``test_performance``, which
    ## the repo's ``test-perf`` recipe builds ``-d:release``.
    source: string
    binary: string
    consumesIsonim: bool
    releaseBuild: bool

const testSpecs: seq[GpuiTestSpec] = @[
  # ---- Leaf tests (self src + the Rust shim; no isonim) --------------
  GpuiTestSpec(source: "tests/test_basic.nim",
    binary: "build/test-bin/test_basic",
    consumesIsonim: false, releaseBuild: false),
  GpuiTestSpec(source: "tests/test_bindings.nim",
    binary: "build/test-bin/test_bindings",
    consumesIsonim: false, releaseBuild: false),
  GpuiTestSpec(source: "tests/test_renderer.nim",
    binary: "build/test-bin/test_renderer",
    consumesIsonim: false, releaseBuild: false),
  GpuiTestSpec(source: "tests/test_performance.nim",
    binary: "build/test-bin/test_performance",
    consumesIsonim: false, releaseBuild: true),
  GpuiTestSpec(source: "tests/test_gui.nim",
    binary: "build/test-bin/test_gui",
    consumesIsonim: false, releaseBuild: false),
  # ---- Consumer tests (isonim + nim-everywhere via SC-11 uses:) ------
  GpuiTestSpec(source: "tests/test_cross_renderer.nim",
    binary: "build/test-bin/test_cross_renderer",
    consumesIsonim: true, releaseBuild: false),
  GpuiTestSpec(source: "tests/test_render_integration.nim",
    binary: "build/test-bin/test_render_integration",
    consumesIsonim: true, releaseBuild: false),
  GpuiTestSpec(source: "tests/test_structural_comparison.nim",
    binary: "build/test-bin/test_structural_comparison",
    consumesIsonim: true, releaseBuild: false),
]

package isonim_gpui:
  defaultToolProvisioning "path"

  uses:
    # Toolchain floor — the PATH-resolvable binaries the build needs.
    # ``nim`` compiles every test binary (the ``buildNimUnittest.build``
    # edges below, matching the nimble file's ``requires "nim >= 2.0.0"``);
    # ``gcc`` is the C back-end ``nim c`` shells out to and links through;
    # ``cargo`` (+ its bundled ``rustc``) builds the ``gpui-nim-shim``
    # cdylib. Sufficient for the path-mode resolver under ``nix develop``.
    "nim >=2.0"
    "gcc >=12"
    "cargo >=1.85"

    # Sibling Nim-library producers (SC-11 develop-mode from-source
    # consumption). The cross-renderer / integration / structural tests
    # ``import isonim/{testing/mock_dom,renderers/terminal_demo,core/*}``;
    # ``isonim/core/platform`` re-exports ``nim_everywhere/platform``, so
    # ``nim-everywhere`` is a direct compile input too. Naming both
    # workspace projects here makes reprobuild build each from source (its
    # ``library isonim`` / ``library nim_everywhere``) and thread its
    # ``src/`` root onto this repo's ``nim c --path:`` via the
    # ``nimPathDirs`` aux channel — replacing the ``nim.cfg`` /
    # ``Justfile`` hardcoded ``--path:../isonim/src``.
    "isonim"
    "nim-everywhere"

  # Library declaration — the ``src/`` tree (``src/isonim_gpui/{bindings,
  # renderer,window}.nim``) is importable when this package is consumed via
  # ``uses: "isonim_gpui"`` (e.g. the ``isonim-examples`` GPUI composition
  # root). Consumers ``import isonim_gpui/renderer`` etc.
  library isonim_gpui

  build:
    # ---- Native cargo build of the gpui-nim-shim cdylib --------------
    #
    # DEFAULT (feature-less) cargo profile → the stub shadow-tree +
    # extern-"C" API, no ``gpui``/``gpui_platform`` git deps, no GPU. The
    # cargo workspace root is ``rust/Cargo.toml`` (the repo root has no
    # ``Cargo.toml``); pin the manifest so cargo resolves it from the
    # recipe CWD (the repo root). ``rust/Cargo.lock`` IS tracked, so
    # ``locked = true`` fails fast rather than silently regenerating the
    # lock. The cdylib lands at ``rust/target/debug/<dylib>`` (cargo's
    # target dir is next to the manifest); declared as ``extraOutputs`` so
    # the engine tracks it as the build edge's effect and every downstream
    # test compile can depend on it.
    const dylibName =
      when defined(windows): "gpui_nim_shim.dll"
      elif defined(macosx): "libgpui_nim_shim.dylib"
      else: "libgpui_nim_shim.so"
    const shimLib = "rust/target/debug/" & dylibName
    const cargoManifest = "rust/Cargo.toml"
    const cargoLockfile = "rust/Cargo.lock"

    let shimBuild = cargo.build(
      locked = true,
      manifestPath = cargoManifest,
      actionId = "isonim-gpui.cargo-build",
      extraInputs = @[
        cargoManifest, cargoLockfile,
        "rust/gpui-nim-shim/Cargo.toml",
        "rust/gpui-nim-shim/src",
      ],
      extraOutputs = @[shimLib])
    discard collect("default", @[shimBuild])

    # ---- Test corpus (the `test` / `test-builds` collections) --------
    #
    # Two-edge template per file (Package-Model.md §"The test template"):
    # a compile-only BUILD edge (collected into ``test-builds``) + an
    # EXECUTE edge (collected into ``test``). Every BUILD edge runs
    # ``after`` the cargo build and declares ``shimLib`` as an
    # ``extraInput`` so the ``.so`` exists at compile time (``bindings.nim``
    # resolves its absolute ``dynlib`` const via ``fileExists``); every
    # EXECUTE edge lists ``shimLib`` via ``requiredBinaries`` so the
    # engine rebuilds+reruns the test when the shim content changes and the
    # binary ``dlopen``s it by absolute path at run time (no
    # ``LD_LIBRARY_PATH`` needed).
    #
    # ``paths = @["src"]`` supplies the repo's own ``--path:src`` (from
    # ``nim.cfg``); the ``../isonim/src`` root ``nim.cfg`` also lists is
    # threaded by the SC-11 ``uses: "isonim"`` / ``uses: "nim-everywhere"``
    # edges, so it is NOT spelled here.
    var testBuildActions: seq[BuildActionDef] = @[]
    var testExecuteActions: seq[BuildActionDef] = @[]

    for spec in testSpecs:
      let stem = splitFile(spec.binary).name
      let buildDefines = if spec.releaseBuild: @["release"] else: newSeq[string]()

      let edge = buildNimUnittest.build(
        source = spec.source,
        binary = spec.binary,
        defines = buildDefines,
        paths = @["src"],
        after = @[shimBuild],
        extraInputs = @["src", "nim.cfg", shimLib],
        actionId = "isonim-gpui.test_build." & stem)
      testBuildActions.add(edge.action)

      # ``registerImplicitName = false`` because the BUILD edge already owns
      # the binary basename as the implicit target name; the explicit
      # ``actionId`` is the execute edge's selector (two-edge shape).
      let executeEdge = edge.testBinary.run(
        actionId = "isonim-gpui.test_execute." & stem,
        requiredBinaries = @[shimLib],
        registerImplicitName = false)
      testExecuteActions.add(executeEdge)

    discard collect("test", testExecuteActions)
    discard collect("test-builds", testBuildActions)
