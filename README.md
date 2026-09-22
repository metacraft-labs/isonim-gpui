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

Since EX-M3, the canonical Task Manager demo lives in the
[`isonim-examples`](../isonim-examples/) repo at
`isonim-examples/task_app/main_gpui.nim`. It consumes the shared
`TaskAppVM` (Layer 3) + view template (Layer 2) and only the GPUI-
specific Layer 1 leaves + Layer 4 composition root differ from the
TUI/web flavours.

### Headless mode (no display server)

Builds the UI tree and runs through interactions programmatically,
printing results to stdout:

```bash
# From this repo's dev shell (provides the Rust shim's runtime libs):
just demo-run

# Or from the isonim-examples repo's dev shell:
cd ../isonim-examples
LD_LIBRARY_PATH=../isonim-gpui/rust/target/debug \
  nim c -r task_app/main_gpui.nim
```

### Window mode (requires display server)

First build the Rust shim with the GPUI backend enabled, then compile with
`-d:gpuiGui`:

```bash
just rust-build          # build the shim library
LD_LIBRARY_PATH=rust/target/debug:${LD_LIBRARY_PATH:-} \
  nim c -r -d:gpuiGui --path:../isonim/src --path:../isonim-examples \
  ../isonim-examples/task_app/main_gpui.nim
```

> Window mode requires a running X11 or Wayland display. For headless CI
> environments, use the Xvfb wrapper (see Testing below).

## Testing

### Nim tests

```bash
just test              # core renderer tests
just test-cross        # cross-renderer compatibility with isonim
just test-integration  # render plan integration tests
just test-structural   # structural comparison tests
just test-perf         # performance benchmarks
just test-all          # all of the above + Rust tests
```

The task-manager demo's end-to-end tests live in `isonim-examples/tests/`
(`test_gpui_leaves_end_to_end.nim`); run them via that repo's
`just test` recipe.

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

### GUI tests under a headless display

```bash
just test-gui                    # GUI tests under headless Sway (THE GUI LANE)
just test-gui-record             # ...and record the display to an MP4
just test-gui --stream           # ...and stream it live to mpv
```

This is the only lane that opens a real GPUI window, and — since
RS-M14b — the only one that asserts on **drawn pixels**: the last suite
in `tests/test_gui.nim` builds a scene of three flat colours with exact
sizes and an exact gap, reads the compositor output back with `grim`
(`scripts/wayland-capture-frame.sh`), and checks each colour's pixel
count and bounding box.

Two things follow from that, both measured rather than assumed:

* **Xvfb cannot be used *here*, and the reason is the CAPTURE, not the
  renderer.** This bullet used to say Xvfb "has no DRI3, so wgpu never
  gets a surface and the GPUI window paints nothing". Re-measured
  2026-09-22 with the windowed shim: **it paints.** The X framebuffer
  (read through `Xvfb -fbdir`) goes from 303 non-NUL bytes of 8,297,632
  with no client attached to 5,184,303 with the window up, and the frame
  decoded out of it is the whole front-end at 1440x900. The
  `libEGL warning: DRI3 error` is still printed — that part was observed
  correctly — but the conclusion did not follow: wgpu falls back to a
  software Vulkan device and renders. What Xvfb cannot do is be READ BACK
  by this harness: `grim` speaks `zwlr_screencopy_manager_v1`, a Wayland
  protocol that does not exist on an X display, and the pinned `ffmpeg`
  has no `x11grab` demuxer. So `just test-gui-x11` still refuses — the
  decision is unchanged, only the reason is true now;
  `scripts/xvfb-run-test.sh` stays for X11 work that does not need the
  GPU.
* **Weston cannot be used.** `weston --backend=headless-backend.so`
  advertises no `wl_seat`, and GPUI's Wayland client unwraps that
  `None` at startup. It used to be this harness's default compositor;
  `scripts/wayland-run-test.sh` now defaults to sway and rejects weston
  with that reason.

Headless sway works with the GPU (`gles2`) and with no GPU at all
(`WLR_RENDERER=pixman`), and the captured frames are byte-identical
between the two.

Shutting the window down is the shim's job, not GPUI's: `Application::
run` blocks until the platform loop stops, and GPUI exposes no handle
to stop it from outside. `gpui_quit()` (any thread) and
`gpui_quit_after_ms()` (a bound, armed before launch) are that handle —
see the "Shutdown" section of `rust/gpui-nim-shim/src/window.rs`.

## Project Structure

```
isonim-gpui/
├── flake.nix                      # Nix flake (Rust + Nim + GPU deps)
├── Justfile                       # Build/test commands
├── scripts/
│   ├── xvfb-run-test.sh          # X11 headless runner (NO GPU rendering)
│   ├── wayland-run-test.sh       # headless Sway runner — the GUI lane
│   └── wayland-capture-frame.sh  # grim capture for the pixel assertions
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
└── tests/                        # Nim test suite
```

The Task Manager demo lives in [`isonim-examples`](../isonim-examples/)
since EX-M3 (`isonim-examples/task_app/{gpui/leaves.nim,main_gpui.nim}`).
The canonical shared core (`task_app/core/{vm,views}.nim`) is consumed
by the TUI, web, and GPUI flavours from a single source.

## GPUI Notes

GPUI is Zed's GPU-accelerated UI framework. It uses:
- A hybrid immediate/retained mode rendering model
- GPU-accelerated rendering targeting 120 FPS
- A Tailwind-inspired styling API (`.bg()`, `.flex()`, `.w()`, etc.)
- `div()` as the primary layout primitive (flexbox-based)
- Platform backends: Metal (macOS), Vulkan (Linux), DirectX (Windows)

### Which GPUI crate this repository builds against

Not the `gpui` crate zed-industries publishes. That one is still **0.2.2**
(October 2025) and predates the headless render API this repository needs.
The shim depends on the **`gpui-pre`** family instead — a crates.io
republication of Zed's own Apache-2.0 crates, maintained by a third party
rather than by Zed, currently pinned at `=0.3.5` (a snapshot of
`zed-industries/zed@d89e9c2`).

That is a supply-chain trade with real consequences, including who decides
whether a revision we need gets published at all.
**[`docs/gpui-pin.md`](docs/gpui-pin.md) states the reasons, the costs, and
the procedure for moving the pin — read it before changing the dependency.**
