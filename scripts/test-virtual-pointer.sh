#!/usr/bin/env bash
# Exercise the real Wayland client against a real headless compositor. The
# protocol trace is emitted by libwayland, not a fake pointer implementation.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [ "${1:-}" != "--inside" ]; then
  bash "$root/scripts/build-virtual-pointer.sh"
  exec bash "$root/scripts/wayland-run-test.sh" -- bash "$0" --inside
fi
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
printf 'down\nup\ndown left\nup left\ndown right\nup right\nquit\n' |
  WAYLAND_DEBUG=client "$root/build/virtual-pointer" 1920 1080 2>"$scratch/protocol"
actual="$(sed -nE 's/.*zwlr_virtual_pointer_v1[@#][0-9]+\.button\([0-9]+, ([0-9]+), ([01])\).*/\1:\2/p' "$scratch/protocol")"
expected="$(printf '272:1\n272:0\n272:1\n272:0\n273:1\n273:0')"
if [ "$actual" != "$expected" ]; then
  cat "$scratch/protocol" >&2
  echo "FAIL: pointer button sequence: expected $expected; got $actual" >&2
  exit 1
fi
for verb in down up; do
  if printf '%s banana\n' "$verb" |
    "$root/build/virtual-pointer" 1920 1080 2>"$scratch/error"; then
    echo "FAIL: $verb banana was accepted" >&2
    exit 1
  fi
  grep -q 'unknown button.*banana' "$scratch/error"
done
echo 'virtual-pointer: left, right and invalid-button protocol checks passed'
