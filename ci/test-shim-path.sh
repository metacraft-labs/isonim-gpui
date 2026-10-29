#!/usr/bin/env bash
# Exercise the real loader: an explicit missing shim must not fall back to the
# shared build. The ordinary build is the positive control, with either Cargo
# feature configuration accepted. No mock library is used.
set -euo pipefail
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/gpui-shim-path.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT
case "$(uname -s)" in
Darwin) library=libgpui_nim_shim.dylib; executable=probe ;;
MINGW* | MSYS* | CYGWIN*) library=gpui_nim_shim.dll; executable=probe.exe ;;
*) library=libgpui_nim_shim.so; executable=probe ;;
esac
cat >"$scratch/probe.nim" <<'NIM'
import isonim_gpui/bindings
echo "backend=", gpui_has_window_backend()
NIM
nim c --hints:off --warnings:off --path:"$repo/src" \
	--nimcache:"$scratch/default-cache" -o:"$scratch/default-$executable" \
	"$scratch/probe.nim"
answer="$("$scratch/default-$executable")"
case "$answer" in
backend=0 | backend=1) ;;
*) echo "FAIL: ordinary shim probe did not report a backend: $answer" >&2; exit 1 ;;
esac
missing="$scratch/missing/$library"
nim c --hints:off --warnings:off --path:"$repo/src" \
	--nimcache:"$scratch/override-cache" "-d:gpuiShimPath=$missing" \
	-o:"$scratch/override-$executable" "$scratch/probe.nim"
rc=0
answer="$("$scratch/override-$executable" 2>&1)" || rc=$?
if [ "$rc" -eq 0 ] || [[ "$answer" != *"$missing"* ]]; then
	printf 'FAIL: explicit missing shim was not refused (rc=%s):\n%s\n' "$rc" "$answer" >&2
	exit 1
fi
echo 'shim path: ordinary shim loads; explicit missing shim is refused by name'
