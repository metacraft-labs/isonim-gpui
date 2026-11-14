#!/usr/bin/env bash
# build-virtual-pointer.sh [out] — build `tools/virtual_pointer.c`, a client
# that gives a headless compositor a REAL pointer device (wlroots'
# `zwlr_virtual_pointer_manager_v1`), into `build/virtual-pointer` (or [out]).
#
# Needs the dev shell: `wayland-scanner`, `libwayland-client` and the
# wlr-protocols XML (`pkg-config --variable=pkgdatadir wlr-protocols`).
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="${1:-$root/build/virtual-pointer}"
xml_dir="$(pkg-config --variable=pkgdatadir wlr-protocols)"
xml="$xml_dir/unstable/wlr-virtual-pointer-unstable-v1.xml"
[ -f "$xml" ] || { echo "no wlr-virtual-pointer XML under $xml_dir" >&2; exit 1; }
gen="$(mktemp -d)"
trap 'rm -rf "$gen"' EXIT
wayland-scanner client-header "$xml" "$gen/wlr-virtual-pointer-unstable-v1-client-protocol.h"
wayland-scanner private-code "$xml" "$gen/wlr-virtual-pointer-unstable-v1-protocol.c"
mkdir -p "$(dirname "$out")"
cc -O1 -Wall -I"$gen" -o "$out" "$root/tools/virtual_pointer.c" \
  "$gen/wlr-virtual-pointer-unstable-v1-protocol.c" \
  $(pkg-config --cflags --libs wayland-client)
echo "built $out"
