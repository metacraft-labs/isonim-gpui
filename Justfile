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

# Run demo app tests (requires Rust shim + isonim)
test-demo:
    LD_LIBRARY_PATH=rust/target/debug:${LD_LIBRARY_PATH:-} nim c -r --path:../isonim/src --path:demos/task-manager/src --nimcache:nimcache/test_demo_app tests/test_demo_app.nim

# Run render-plan integration tests (requires Rust shim + isonim)
test-integration:
    LD_LIBRARY_PATH=rust/target/debug:${LD_LIBRARY_PATH:-} nim c -r --path:../isonim/src --nimcache:nimcache/test_render_integration tests/test_render_integration.nim

# Run performance benchmarks (requires Rust shim)
test-perf:
    LD_LIBRARY_PATH=rust/target/debug:${LD_LIBRARY_PATH:-} nim c -r -d:release --path:../isonim/src --nimcache:nimcache/test_performance tests/test_performance.nim

# Build the demo app
demo-build:
    LD_LIBRARY_PATH=rust/target/debug:${LD_LIBRARY_PATH:-} nim c --path:../isonim/src --nimcache:nimcache/demo demos/task-manager/src/main.nim

# Run the demo app (headless mode)
demo-run:
    LD_LIBRARY_PATH=rust/target/debug:${LD_LIBRARY_PATH:-} nim c -r --path:../isonim/src --nimcache:nimcache/demo demos/task-manager/src/main.nim

# Run structural comparison tests (G4 — requires Rust shim and isonim)
test-structural:
    LD_LIBRARY_PATH=rust/target/debug:${LD_LIBRARY_PATH:-} nim c -r --path:../isonim/src --nimcache:nimcache/test_structural_comparison tests/test_structural_comparison.nim

# Run all tests (Rust + Nim + cross-renderer + demo + integration)
test-all: rust-test test test-cross test-demo test-integration test-structural

# Clean build artifacts
clean:
    rm -rf nimcache tests/test_basic tests/test_bindings tests/test_cross_renderer tests/test_demo_app tests/test_performance tests/test_render_integration demos/task-manager/src/main
    cd rust && cargo clean
