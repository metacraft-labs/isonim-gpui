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

# Run all tests (Rust + Nim)
test-all: rust-test test

# Clean build artifacts
clean:
    rm -rf nimcache tests/test_basic tests/test_bindings
    cd rust && cargo clean
