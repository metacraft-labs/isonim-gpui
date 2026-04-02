# IsoNim-GPUI

Nim bindings for [GPUI](https://gpui.rs/), Zed's GPU-accelerated UI framework.
Implements IsoNim's `RendererBackend` concept, enabling cross-renderer
Nim applications that can target GPUI for high-performance desktop rendering.

## Architecture

```
Nim (IsoNim DSL / reactive core)
  │
  v
Nim C bindings  (src/isonim_gpui/bindings.nim)
  │
  v  extern "C" FFI
Rust shim       (rust/gpui-nim-shim/)
  │
  v
GPUI (Zed's GPU-accelerated UI)
```

The Rust shim maintains an in-memory **shadow tree** that mirrors the Nim-side
element tree. A **render plan** converts this tree into GPUI-native elements
via `NimRootView`. The `gpui-backend` Cargo feature gates actual GPUI
rendering; without it the shim provides the shadow tree API only (for testing
and CI).

## Prerequisites

- [Nix](https://nixos.org/) with flakes enabled
- direnv (recommended)
- The `isonim` core library checked out as a sibling: `../isonim/`

## Quick Start

```bash
# Enter dev shell (Rust + Nim + GPU deps)
direnv allow   # or: nix develop

# Build the Rust shim (stub mode, no display server needed)
just rust-build

# Build with GPUI backend (requires GPU/display libs)
just rust-check-gpui

# Run all tests
just test-all
```

## Running the Demo App

The repo includes a **Task Manager** demo at `demos/task-manager/src/main.nim`.
It exercises signals, memos, reactive rendering, event dispatch, and tree
mutations — the same app that runs in the browser via isonim's web renderer.

### Headless mode (no display server)

Builds the UI tree and runs through all interactions programmatically,
printing the results to stdout:

```bash
just demo-run
```

### Window mode (requires display server)

First build the Rust shim with the GPUI backend enabled, then compile with
`-d:gpuiGui`:

```bash
just rust-build          # build the shim library
LD_LIBRARY_PATH=rust/target/debug:${LD_LIBRARY_PATH:-} \
  nim c -r -d:gpuiGui --path:../isonim/src demos/task-manager/src/main.nim
```

> Window mode requires a running X11 or Wayland display. For headless CI
> environments, use the Xvfb wrapper (see Testing below).

## Testing

### Nim tests

```bash
just test              # core renderer tests
just test-cross        # cross-renderer compatibility with isonim
just test-demo         # task manager demo verification
just test-integration  # render plan integration tests
just test-structural   # structural comparison tests
just test-perf         # performance benchmarks
just test-all          # all of the above + Rust tests
```

### Rust tests

```bash
just rust-test                                   # lib + integration tests (stub mode)
cd rust/gpui-nim-shim && cargo test              # same, from crate dir
cd rust/gpui-nim-shim && cargo test --features gpui-backend -- --test-threads=1
```

> Note: with `--features gpui-backend`, use `--test-threads=1` to avoid mutex
> contention on the global shadow tree.

The integration test suite (`rust/gpui-nim-shim/tests/gpui_rendering.rs`)
uses GPUI's `TestAppContext` to render through the actual GPUI pipeline
headlessly — no display server needed.

### GUI tests under headless display

```bash
just test-gui-x11                # run GUI tests under Xvfb
just test-gui-wayland            # run GUI tests under headless Sway
just test-gui-record             # run under Xvfb and record video
just test-gui-x11 --stream       # run under Xvfb with live video stream
```

## Project Structure

```
isonim-gpui/
├── flake.nix                      # Nix flake (Rust + Nim + GPU deps)
├── Justfile                       # Build/test commands
├── scripts/
│   ├── xvfb-run-test.sh          # X11 headless test runner
│   └── wayland-run-test.sh       # Wayland headless test runner
├── rust/
│   └── gpui-nim-shim/
│       ├── src/
│       │   ├── lib.rs            # extern "C" FFI exports
│       │   ├── tree.rs           # Shadow element tree
│       │   ├── render_sync.rs    # Render plan builder
│       │   ├── gpui_app.rs       # GPUI app launcher + NimRootView
│       │   └── window.rs         # Window state machine
│       └── tests/
│           └── gpui_rendering.rs # Integration tests (GPUI TestAppContext)
├── src/isonim_gpui/
│   ├── bindings.nim              # Raw C bindings to Rust shim
│   └── renderer.nim              # GpuiRenderer (RendererBackend impl)
├── tests/                        # Nim test suite
└── demos/task-manager/           # Task Manager demo app
```

## GPUI Notes

GPUI (v0.2.x) is available on crates.io. It uses:
- A hybrid immediate/retained mode rendering model
- GPU-accelerated rendering targeting 120 FPS
- A Tailwind-inspired styling API (`.bg()`, `.flex()`, `.w()`, etc.)
- `div()` as the primary layout primitive (flexbox-based)
- Platform backends: Metal (macOS), Vulkan (Linux), DirectX (Windows)
