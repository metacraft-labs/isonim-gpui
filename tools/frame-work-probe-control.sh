#!/usr/bin/env bash
# The NEGATIVE CONTROL of `rust/gpui-nim-shim/tests/frame_work_probe.rs`.
#
# Runs the probe, unchanged, against the shim as it was before the render plan
# became incremental: the commit given (default: 2db29d9, the last commit
# before it) with `frame-work-probe-control.patch` applied, which adds the
# probe's counters (`gpui_frame_work`) to the old full walk and changes
# nothing else. There every frame visits and rebuilds every shadow node, so
# the probe must FAIL, with its counts scaling with the rows. This script
# exits 0 when it does and 1 when the probe passed, which would mean the
# probe cannot tell the two shims apart.
#
# Usage: tools/frame-work-probe-control.sh [base-commit]
# Needs the dev shell (cargo) and builds in a temporary worktree, into the
# `frame-work-control` directory under CARGO_TARGET_DIR (default rust/target),
# which later runs reuse.

set -uo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BASE="${1:-2db29d9}"
WT="$(mktemp -d "${TMPDIR:-/tmp}/frame-work-control.XXXXXX")"
cleanup() { git -C "$REPO_ROOT" worktree remove --force "$WT" >/dev/null 2>&1; rm -rf "$WT"; }
trap cleanup EXIT

git -C "$REPO_ROOT" worktree add -q --detach "$WT" "$BASE" || exit 2
git -C "$WT" apply "$REPO_ROOT/tools/frame-work-probe-control.patch" || exit 2
cp "$REPO_ROOT/rust/gpui-nim-shim/tests/frame_work_probe.rs" "$WT/rust/gpui-nim-shim/tests/" || exit 2

# A target directory of its own: cargo identifies a workspace's packages by
# their path RELATIVE to the workspace, so the old shim built into the
# caller's target directory would look up to date to the caller's next build.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/rust/target}/frame-work-control"
out="$(cd "$WT/rust" && cargo test -p gpui-nim-shim --features gpui-backend --test frame_work_probe -- --nocapture 2>&1)"
rc=$?
printf '%s\n' "$out" | grep -E '^probe |panicked|grew with the rows|bound [0-9]|test result' || true
if [ "$rc" -eq 0 ]; then
	echo "CONTROL FAILED: the probe passed against the pre-change shim at $BASE" >&2
	exit 1
fi
if ! printf '%s\n' "$out" | grep -q 'frame_work_probe.rs'; then
	echo "CONTROL INCONCLUSIVE: the probe did not run (rc=$rc); see the output above" >&2
	exit 2
fi
echo "control holds: the probe fails against the pre-change shim at $BASE (rc=$rc)"
