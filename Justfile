# isonim-gpui build commands

# Check that the Rust shim compiles (stub mode, no GPUI backend)
rust-check:
    cd rust && cargo check

# Check that the Rust shim compiles with GPUI backend
rust-check-gpui:
    cd rust && cargo check --features gpui-backend

# Build the Rust shim as a cdylib
rust-build:
    cd rust && cargo build

# Build the Rust shim in release mode
rust-build-release:
    cd rust && cargo build --release

# Check that the Nim code compiles
nim-check:
    nim c --nimcache:nimcache/test_basic tests/test_basic.nim

# Check that the renderer compiles
nim-check-renderer:
    nim check --nimcache:nimcache/check_renderer src/isonim_gpui/renderer.nim

# Run Rust tests
rust-test:
    cd rust && cargo test

# Run Nim tests (requires Rust shim to be built: just rust-build)
test:
    nim c -r --hints:off tests/test_nimcache_is_worktree_local.nim
    LD_LIBRARY_PATH=rust/target/debug:${LD_LIBRARY_PATH:-} nim c -r --nimcache:nimcache/test_basic tests/test_basic.nim

# Check that the bindings compile (compile-time verification of all 40 symbols)
nim-check-bindings:
    nim c --nimcache:nimcache/test_bindings tests/test_bindings.nim

# Generate Nim bindings from Rust shim using nbindgen
generate-bindings:
    ./tools/generate_bindings.sh

# Check that all Rust exports have matching Nim bindings.
#
# SOURCE-LEVEL. This reads `extern "C"` declarations out of the crate's
# .rs files and is `cfg`-blind by construction, so it cannot see a
# declaration that the default feature selection compiles out. Run
# `check-exported-symbols` for that; the two are complements, not
# alternatives, and only the second one can catch a "could not import:"
# at process load.
check-bindings:
    ./tools/check_bindings.sh

# Check that every symbol bindings.nim imports is EXPORTED BY A BUILT
# SHIM. Builds the default (feature-less) profile itself and reads
# `nm -D` off it — see the script header and
# `codetracer-specs/Testing/Verification-Harness-Traps.md` §18.
check-exported-symbols:
    ./tools/check_exported_symbols.sh

# Run cross-renderer tests (requires Rust shim + isonim)
test-cross:
    LD_LIBRARY_PATH=rust/target/debug:${LD_LIBRARY_PATH:-} nim c -r --path:../isonim/src --nimcache:nimcache/test_cross_renderer tests/test_cross_renderer.nim

# Run render-plan integration tests (requires Rust shim + isonim)
test-integration:
    LD_LIBRARY_PATH=rust/target/debug:${LD_LIBRARY_PATH:-} nim c -r --path:../isonim/src --nimcache:nimcache/test_render_integration tests/test_render_integration.nim

# Run performance benchmarks (requires Rust shim)
test-perf:
    LD_LIBRARY_PATH=rust/target/debug:${LD_LIBRARY_PATH:-} nim c -r -d:release --path:../isonim/src --nimcache:nimcache/test_performance tests/test_performance.nim

# Build the canonical task-app demo (lives in isonim-examples since
# EX-M3; this recipe just defers to that repo's composition root).
demo-build:
    LD_LIBRARY_PATH=rust/target/debug:${LD_LIBRARY_PATH:-} nim c --path:../isonim/src --path:../isonim-examples --path:../isonim-examples/src --nimcache:nimcache/demo ../isonim-examples/task_app/main_gpui.nim

# Run the canonical task-app demo (headless mode). Sources live in
# `isonim-examples/task_app/` per the EX-M3 migration; the Rust shim
# is still built in this repo.
demo-run:
    LD_LIBRARY_PATH=rust/target/debug:${LD_LIBRARY_PATH:-} nim c -r --path:../isonim/src --path:../isonim-examples --nimcache:nimcache/demo ../isonim-examples/task_app/main_gpui.nim

# Run structural comparison tests (G4 — requires Rust shim and isonim)
test-structural:
    LD_LIBRARY_PATH=rust/target/debug:${LD_LIBRARY_PATH:-} nim c -r --path:../isonim/src --nimcache:nimcache/test_structural_comparison tests/test_structural_comparison.nim

# NH-M1: the reactive root seam (`renderGpui`). Requires the Rust shim —
# every `gpui_*` binding is `dynlib`, so without `just rust-build` the binary
# does not start ("could not load: libgpui_nim_shim.so"). That hard failure is
# deliberate: there is no skip arm, because a suite that went green without
# the shim would be asserting nothing about GPUI.
test-reactive-root:
    LD_LIBRARY_PATH=rust/target/debug:${LD_LIBRARY_PATH:-} nim c -r --path:../isonim/src --nimcache:nimcache/test_render_native_gpui_reactive tests/test_render_native_gpui_reactive.nim

# Run all tests (Rust + Nim + cross-renderer + integration). The
# task-manager demo's tests live in `isonim-examples/tests/` since
# EX-M3 (`test_gpui_leaves_end_to_end.nim`); run them via that repo's
# `just test` recipe.
# NH-M3: the per-renderer reconciler instance and its identity gate.
# Requires the Rust shim (`just rust-build`) for the same reason
# `test-reactive-root` does: every `gpui_*` binding is `dynlib`, so
# without it the binary does not start. No skip arm — a suite that went
# green without the shim would be asserting nothing about Gpui.
test-reconciler:
    LD_LIBRARY_PATH=rust/target/debug:${LD_LIBRARY_PATH:-} nim c -r --path:../isonim/src -d:isonimHmr --nimcache:nimcache/test_gpui_reconciler tests/test_gpui_reconciler_identity.nim

test-all: check-exported-symbols rust-test test test-cross test-integration test-structural test-reactive-root test-reconciler

# Run the GUI tests, including the windowed pixel case, under headless
# sway. THIS IS THE GUI LANE. (RS-M14b)
#
# `test-gui-wayland` is kept as an alias because CI and muscle memory
# both refer to it.
# THE WINDOWED SHIM IS RESTORED ON THE WAY OUT, AS `test-gui-keyboard` DOES.
# `rust/target/debug/libgpui_nim_shim.so` is ONE path that this recipe and
# several sibling suites read with different feature selections, so a run
# that ended leaving the `gpui-backend` build behind would make the next,
# unrelated suite fail for THIS recipe's reason — Verification-Harness-Traps
# §18, *pin the image, or the manifest measures whichever build ran last*.
# PLAT-38 fixed this for `test-gui-keyboard` and recorded the same hazard
# here; closing it is PLAT-37's, and this is it.
test-gui *ARGS:
    #!/usr/bin/env bash
    set -uo pipefail
    (cd rust && cargo build --features gpui-backend) || exit $?
    ./scripts/wayland-run-test.sh {{ARGS}} just _run-gui-tests
    rc=$?
    # Restored WHATEVER the outcome, for the reason above.
    (cd rust && cargo build) || true
    exit $rc

test-gui-wayland *ARGS:
    just test-gui {{ARGS}}

# PLAT-42 — CAN gpui-kit BE MEASURED AT ALL? A two-sided compile probe.
#
# PLAT-22 declined to measure gpui-kit's editor and gave three reasons. The
# FIRST was that the package split forbids linking: gpui-kit depends on
# `gpui-pre`, we depended on `gpui` from zed's git remote, and same lib name +
# different package name = different types, so no gpui-kit component could ever
# be handed one of our windows.
#
# This recipe tests that reason instead of repeating it, in BOTH directions,
# because a compile that succeeds proves unification only if the same probe
# FAILS on a split graph:
#
#   positive  `cargo build -p gpui-kit-probe` — values built through OUR gpui
#             path are passed to gpui-base and gpui-component, and a value they
#             RETURN is consumed as ours. Must succeed.
#   negative  the same probe with our `gpui` pointed at the real `gpui` 0.2.2
#             package. Must FAIL, with rustc naming two `Pixels` types.
#
# Measured 2026-09-23: positive rc 0; negative rc 101, "expected
# `gpui::geometry::Pixels`, found `gpui::Pixels`". So the first reason is
# REMOVED and candidates A and B are measurable.
#
# What it does NOT claim: that gpui-kit's editor is adoptable. The first draft
# of the probe reached for `gpui_base::touch_selection::TouchSelection` and was
# refused because that module is PRIVATE — evidence about candidate A's sealed
# surface, not about linking, and the two must not be conflated.
plat42-gpui-kit-probe:
    cd rust && cargo build -p gpui-kit-probe

# PLAT-37 (codetracer) — the THREE shim configurations, side by side.
#
# `rust/target/debug/libgpui_nim_shim.so` is ONE path that several recipes in
# this repo and two sibling repos write to with different feature selections,
# so a consumer that merely read whatever was lying there would be measuring
# whichever build ran last — `codetracer-specs/Testing/Verification-Harness-
# Traps.md` §18's second corollary, *pin the image, or the manifest measures
# whichever build ran last*. This recipe produces all three under names that
# say which configuration each one is, and leaves the shared `target/` cache
# to do the heavy lifting (the gpui dependency graph is built once).
#
# WHY IT LIVES HERE AND NOT IN `codetracer`. Linking the windowed shim needs
# `-lxcb`, `-lxkbcommon` and `-lxkbcommon-x11` on the LINKER's search path,
# and this repo's `flake.nix` is where those are declared. Measured
# 2026-09-22: `cargo build --features gpui-backend` run from codetracer's dev
# shell fails with three `rust-lld: error: unable to find library` lines. A
# cross-repo build is the sibling's own recipe, invoked through its own shell
# — the same arrangement `codetracer/scripts/build-siblings.sh` already uses
# for the recorders.
#
#   featureless  the default profile. `default = []`, the shadow-tree shim,
#                no `gpui` dependency at all. codetracer's DIFF-6 control.
#   windowed     `--features gpui-backend`. `gpui_launch` enters
#                `Application::run` and opens a real window.
#   headless     `--features gpui-headless`. `gpui_render_to_pixels` over
#                `HeadlessAppContext` + `Window::render_to_image`. No
#                compositor, and NOT a substitute for the windowed path.
plat37-shims:
    #!/usr/bin/env bash
    set -euo pipefail
    out="rust/target/plat37"
    mkdir -p "$out"
    for spec in "featureless:" "windowed:--features gpui-backend" "headless:--features gpui-headless"; do
      config="${spec%%:*}"
      flags="${spec#*:}"
      echo "=== ${config} ${flags:-(default)} ==="
      # shellcheck disable=SC2086  # the flag string must word-split
      (cd rust && cargo build ${flags})
      cp rust/target/debug/libgpui_nim_shim.so "$out/libgpui_nim_shim.${config}.so"
      ldd "$out/libgpui_nim_shim.${config}.so" > "$out/${config}.ldd.txt" 2>&1 || true
      nm -D --defined-only "$out/libgpui_nim_shim.${config}.so" \
        | awk '{print $NF}' | grep '^gpui_' | sort -u > "$out/${config}.symbols.txt"
      echo "  $(wc -c <"$out/libgpui_nim_shim.${config}.so") bytes, \
    $(wc -l <"$out/${config}.ldd.txt") ldd entries, \
    $(wc -l <"$out/${config}.symbols.txt") gpui_ symbols"
    done
    # The default profile is left in `target/debug`, because that is the one
    # every OTHER consumer of this repo expects to find there. A recipe that
    # left a 544 MB windowed cdylib at the shared path would change what the
    # 237-case lane and both sibling repos link, silently.
    (cd rust && cargo build)
    echo "OK: three shims in ${out}"

# Run the GUI tests with video recording.
test-gui-record:
    just test-gui --record

# THE X11/XVFB PATH CANNOT BE CAPTURED HERE, AND THIS RECIPE SAYS SO
# RATHER THAN RUNNING.
#
# THIS BLOCK SAID "CANNOT RENDER" UNTIL 2026-09-22 AND THAT WAS WRONG.
# The 2026-09-17 reading was that under Xvfb the GPUI window reaches
# `IsViewable` and paints nothing "because Xvfb has no DRI3 and wgpu
# therefore never gets a surface". Re-measured against the windowed shim:
# **it paints.** The `Xvfb -fbdir` framebuffer goes from 303 non-NUL bytes
# of 8,297,632 with no client to 5,184,303 with the window up, and the
# frame decoded out of it is the whole front-end. The `libEGL DRI3`
# warning is still emitted; wgpu falls back to a software Vulkan device.
# The 2026-09-17 reading was taken with a capture that cannot see an X
# root window, which is why it read blank.
#
# THE RECIPE STILL REFUSES, for the reason that survives: this harness
# reads frames back with `grim`, a `zwlr_screencopy_manager_v1` client,
# and that protocol does not exist on an X display; the pinned `ffmpeg`
# has no `x11grab` demuxer either. The pass-shaped failure the pixel case
# in `tests/test_gui.nim` exists to end is still real — an assertion over
# the shadow tree, the render plan and the window state machine passes
# whether or not anything reached a display — it is just not Xvfb's fault.
#
# It is a `just` recipe rather than a deleted line because
# `.github/workflows/ci.yml` used to run `just test-gui-x11`, and a
# recipe that has quietly become a no-op is worse than one that is gone.
test-gui-x11:
    #!/usr/bin/env bash
    echo "test-gui-x11 is not supported: this harness reads frames back with" >&2
    echo "grim, a zwlr_screencopy_manager_v1 client, and that protocol does" >&2
    echo "not exist on an X display (the pinned ffmpeg has no x11grab either)." >&2
    echo "NOTE: the window itself DOES paint under Xvfb -- re-measured" >&2
    echo "2026-09-22, 303 -> 5,184,303 non-NUL framebuffer bytes. The old" >&2
    echo "message here blamed DRI3 and was wrong. Use 'just test-gui'," >&2
    echo "which runs under headless sway and asserts on captured pixels." >&2
    exit 1

# Internal: actual GUI test commands (run inside headless display)
_run-gui-tests:
    LD_LIBRARY_PATH=rust/target/debug:${LD_LIBRARY_PATH:-} nim c -r --nimcache:nimcache/test_gui -d:gpuiBackend --path:../isonim/src tests/test_gui.nim

# PLAT-38 — A REAL KEY, through the compositor's own `wl_seat`, into a
# focused GPUI window, read back from the Rust-side element store.
#
# It is a SEPARATE recipe from `test-gui` on purpose. That lane's subject is
# PIXELS: `wayland-capture-frame.sh` waits for the output to go blank, then
# for a painted-and-settled frame, and asserts exact colour counts. A typist
# sending keys into the same compositor would change the screen while the
# capture was deciding whether it had settled — two experiments on one
# display, each able to fail the other for a reason belonging to neither.
#
# The shim MUST be the windowed one. `--features gpui-backend` is what makes
# `gpui_launch` enter `Application::run`; without it the window never opens,
# no compositor key can reach anything, and the suite refuses at compile time
# rather than reporting an empty population.
#
# **IT RESTORES THE DEFAULT SHIM WHEN IT IS DONE, AND THAT IS NOT TIDINESS.**
# `rust/target/debug/libgpui_nim_shim.so` is baked as an ABSOLUTE `dynlib`
# path into every Nim binary in this workspace at COMPILE time, so a recipe
# that left the WINDOWED build there would break every sibling suite that
# links the shim — `codetracer`'s dev shell carries no `libxcb` /
# `libxkbcommon`, so those binaries die with `could not load:` rather than
# with anything that names a cause. Measured 2026-09-22: PLAT-38's floor gate
# failed exactly that way after this recipe ran. `test-gui` has the same
# shape and the same hazard; it is left alone here because changing it is
# PLAT-37's to do, and the reason is recorded rather than fixed twice.
test-gui-keyboard *ARGS:
    #!/usr/bin/env bash
    set -uo pipefail
    (cd rust && cargo build --features gpui-backend) || exit $?
    ./scripts/wayland-run-test.sh {{ARGS}} just _run-gui-keyboard
    rc=$?
    # Restored WHATEVER the outcome: a failed run that left the windowed shim
    # behind would make the next unrelated suite fail for this recipe's reason.
    (cd rust && cargo build) || true
    exit $rc

_run-gui-keyboard:
    LD_LIBRARY_PATH=rust/target/debug:${LD_LIBRARY_PATH:-} nim c -r --nimcache:nimcache/test_gui_keyboard -d:gpuiBackend --path:../isonim/src tests/test_gui_keyboard.nim

# Clean build artifacts
clean:
    rm -rf nimcache tests/test_basic tests/test_bindings tests/test_cross_renderer tests/test_performance tests/test_render_integration
    cd rust && cargo clean
