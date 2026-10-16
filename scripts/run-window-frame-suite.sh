#!/usr/bin/env bash
# run-window-frame-suite.sh — the window-frame suite end to end.
#
#   1. In THIS repo's dev shell: the windowed shim (`--features gpui-backend`,
#      target `rust/target/windowed`) and `tests/gui/window_frame_app.nim`
#      pinned to it; the shell's LD_LIBRARY_PATH (the GPU and Wayland
#      libraries the shim links) is recorded.
#   2. In GuiAssert's dev shell (`$GUI_ASSERT_DIR`, default ../GuiAssert),
#      which pins sway and its session stack: `tests/test_gui_window_frame.nim`,
#      which spawns the app with that library path.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
guiassert="$(cd "${GUI_ASSERT_DIR:-$root/../GuiAssert}" && pwd)"
[ -f "$guiassert/src/gui_assert/desktop_env/sway.nim" ] ||
	{ echo "no GuiAssert checkout with desktop_env at $guiassert" >&2; exit 2; }
target="$root/rust/target/windowed"
mkdir -p "$root/build"
libs="$root/build/window-frame-app.ld-path"
(cd "$root" && nix develop --command bash -c "
	set -euo pipefail
	(cd rust && CARGO_TARGET_DIR='$target' cargo build -q --features gpui-backend)
	nim c --hints:off --warnings:off --path:src \
		-d:gpuiShimPath='$target/debug/libgpui_nim_shim.so' \
		--nimcache:'$root/nimcache/window_frame_app' \
		-o:'$root/build/window_frame_app' tests/gui/window_frame_app.nim
	printf '%s' \"\$LD_LIBRARY_PATH\" > '$libs'")
(cd "$guiassert" && GWIN_APP="$root/build/window_frame_app" \
	GWIN_APP_LD_LIBRARY_PATH="$(cat "$libs")" nix develop --command \
	nim c -r --hints:off --warnings:off --path:src \
	--nimcache:"$root/nimcache/test_gui_window_frame" \
	-o:"$root/build/test_gui_window_frame" "$root/tests/test_gui_window_frame.nim")
