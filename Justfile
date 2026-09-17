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
    LD_LIBRARY_PATH=rust/target/debug:${LD_LIBRARY_PATH:-} nim c -r --nimcache:nimcache/test_basic tests/test_basic.nim

# Check that the bindings compile (compile-time verification of all 40 symbols)
nim-check-bindings:
    nim c --nimcache:nimcache/test_bindings tests/test_bindings.nim

# Generate Nim bindings from Rust shim using nbindgen
generate-bindings:
    ./tools/generate_bindings.sh

# Check that all Rust exports have matching Nim bindings
check-bindings:
    ./tools/check_bindings.sh

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

# Run all tests (Rust + Nim + cross-renderer + integration). The
# task-manager demo's tests live in `isonim-examples/tests/` since
# EX-M3 (`test_gpui_leaves_end_to_end.nim`); run them via that repo's
# `just test` recipe.
test-all: rust-test test test-cross test-integration test-structural

# Run the GUI tests, including the windowed pixel case, under headless
# sway. THIS IS THE GUI LANE. (RS-M14b)
#
# `test-gui-wayland` is kept as an alias because CI and muscle memory
# both refer to it.
test-gui *ARGS:
    cd rust && cargo build --features gpui-backend
    ./scripts/wayland-run-test.sh {{ARGS}} just _run-gui-tests

test-gui-wayland *ARGS:
    just test-gui {{ARGS}}

# Run the GUI tests with video recording.
test-gui-record:
    just test-gui --record

# THE X11/XVFB PATH CANNOT RENDER, AND THIS RECIPE SAYS SO RATHER THAN
# RUNNING. Measured 2026-09-17: under Xvfb the GPUI window reaches
# `IsViewable` at its requested size and paints nothing, because Xvfb has
# no DRI3 and wgpu therefore never gets a surface. Every assertion over
# the shadow tree, the render plan and the window state machine passes on
# that — which is exactly the pass-shaped failure the pixel case in
# `tests/test_gui.nim` exists to end.
#
# It is a `just` recipe rather than a deleted line because
# `.github/workflows/ci.yml` used to run `just test-gui-x11`, and a
# recipe that has quietly become a no-op is worse than one that is gone.
test-gui-x11:
    #!/usr/bin/env bash
    echo "test-gui-x11 is not supported: Xvfb has no DRI3, so wgpu gets no" >&2
    echo "surface and the GPUI window paints nothing. Use 'just test-gui'," >&2
    echo "which runs under headless sway and asserts on captured pixels." >&2
    exit 1

# Internal: actual GUI test commands (run inside headless display)
_run-gui-tests:
    LD_LIBRARY_PATH=rust/target/debug:${LD_LIBRARY_PATH:-} nim c -r --nimcache:nimcache/test_gui -d:gpuiBackend --path:../isonim/src tests/test_gui.nim

# Clean build artifacts
clean:
    rm -rf nimcache tests/test_basic tests/test_bindings tests/test_cross_renderer tests/test_performance tests/test_render_integration
    cd rust && cargo clean
