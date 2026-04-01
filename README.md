# IsoNim-GPUI

Nim bindings for [GPUI](https://gpui.rs/), Zed's GPU-accelerated UI framework.
Implements IsoNim's `RendererBackend` concept, enabling cross-renderer
Nim applications that can target GPUI for high-performance desktop rendering.

## Prerequisites

- [Nix](https://nixos.org/) with flakes enabled
- direnv (recommended)

## Quick Start

```sh
# Enter dev shell
direnv allow   # or: nix develop

# Check Rust shim compiles (stub mode, no actual GPUI)
just rust-check

# Check Nim bindings compile
just nim-check

# Build Rust shim with GPUI backend
just rust-check-gpui

# Run all tests
just test-all
```

## Project Structure

```
isonim-gpui/
├── flake.nix              # Nix flake with Rust + Nim + GPUI deps
├── Justfile               # Build/test commands
├── rust/
│   ├── Cargo.toml         # Workspace root
│   └── gpui-nim-shim/     # Rust FFI shim crate
│       ├── Cargo.toml
│       └── src/lib.rs
├── src/
│   └── isonim_gpui/
│       ├── bindings.nim   # Raw C bindings to Rust shim
│       └── renderer.nim   # GpuiRenderer (RendererBackend impl)
└── tests/
    └── test_basic.nim     # Compile-time conformance test
```

## Architecture

The binding follows the same pattern as `isonim-freya`:

1. **Shadow tree** in Rust — imperative tree manipulation via `extern "C"` functions
2. **Nim bindings** — `{.cdecl, dynlib.}` imports matching the Rust exports
3. **GpuiRenderer** — maps HTML-like tags/CSS-like styles to GPUI's element model
4. **Feature flag** — `gpui-backend` enables actual GPUI rendering; without it,
   the shim provides the shadow tree API only (for testing and CI)

## GPUI Notes

GPUI (v0.2.x) is available on crates.io. It uses:
- A hybrid immediate/retained mode rendering model
- GPU-accelerated rendering targeting 120 FPS
- A Tailwind-inspired styling API (`.bg()`, `.flex()`, `.w()`, etc.)
- `div()` as the primary layout primitive (flexbox-based)
- Platform backends: Metal (macOS), Vulkan (Linux), DirectX (Windows)
