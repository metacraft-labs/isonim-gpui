#!/usr/bin/env bash
# PLAT-19 — the isonim-gpui test lane.
#
# Before this existed, `.github/workflows/ci.yml` listed the suites as
# fifteen separate `run:` steps and could never have executed them: its
# `nim-tests` job checks isonim out with `path: ../isonim`, which
# actions/checkout REFUSES because the path is outside $GITHUB_WORKSPACE,
# and the `Validate Rust/Nim binding consistency` step was red at HEAD
# for six symbols. A lane made of fifteen steps that nobody has seen
# green is the shape `vm-js` had when this campaign found it.
#
# WHAT THIS LANE REPORTS, AND WHY IT REPORTS IT
# ---------------------------------------------
# Three numbers, and they are not interchangeable:
#
#   * THE VERDICT IS THE EXIT CODE. A case count is not a pass signal.
#     Measured on 2026-09-15 in this repository: with a trap-13 defect
#     present (`check` inside a plain `proc`), `tests/test_gui.nim`
#     printed SEVEN `Check failed:` lines — one per `getPlan` call site
#     — and fourteen `[OK]`s and not one `[FAILED]`. (An earlier draft
#     of this comment said eight, which is the `grep -c` hit count
#     including the declaration, not the run.) An `[OK]`-counting lane
#     would have reported 14/14.
#     So every subject's rc is captured directly — never through a pipe,
#     because `cmd | tail` yields tail's status, which is how a baseline
#     run of this very suite reported `RC=0` over three subjects that
#     did not compile.
#
#   * THE CASE COUNT IS THE ANTI-SILENT-SKIP FINGERPRINT (traps 4b/4c).
#     A subject that stops running does not fail; it ceases to exist, and
#     the run stays green. The lane totals the cases and requires the
#     total to equal `EXPECTED_CASES` below, so a suite that vanished is
#     red by name. That number is written LAST, from a run.
#
#   * THE SUBSET IS NAMED (PLAT-17's rule). By default this lane runs
#     everything except the two Cargo feature builds that link GPUI
#     itself, because those pull ~500 crates of Zed. `--with-gpui` runs
#     them. Either way the lane PRINTS which of the two it did, so a run
#     that skipped the GPUI-linking half cannot be read as a full one.
#
# Usage:
#   ci/run-suite.sh                # core lane
#   ci/run-suite.sh --with-gpui    # core lane + the GPUI-linking builds
#
# Must be run inside the dev shell (`direnv exec .` / `nix develop -c`).

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT" || exit 2

WITH_GPUI=0
for arg in "$@"; do
	case "$arg" in
	--with-gpui) WITH_GPUI=1 ;;
	*)
		echo "unknown argument: $arg" >&2
		exit 2
		;;
	esac
done

export LD_LIBRARY_PATH="$REPO_ROOT/rust/target/debug:${LD_LIBRARY_PATH:-}"
NIMCACHE_ROOT="${NIMCACHE_ROOT:-$REPO_ROOT/nimcache/lane}"

# --- The subjects. -----------------------------------------------------
#
# `unittest`-based suites print `[OK] <name>` / `[FAILED] <name>` per
# case and are counted that way. The three assert-style scripts print one
# `name: … passed` checkpoint per block instead and have no unittest
# cases at all; they are counted by checkpoint and LABELLED as such,
# rather than being silently counted as zero — a zero that is
# indistinguishable from a suite that did not run is the whole defect
# this lane exists to make impossible.
UNITTEST_SUITES=(
	tests/test_window_callback_registry.nim
	tests/test_cross_renderer.nim
	tests/test_gui.nim
	tests/test_render_integration.nim
	tests/test_structural_comparison.nim
)
ASSERT_SCRIPTS=(
	tests/test_basic.nim
	tests/test_bindings.nim
	tests/test_renderer.nim
)
# test_performance.nim is a BENCHMARK, not a test: it prints timings and
# asserts nothing about them. It is compiled and run by the lane so a
# benchmark that stopped building is red, and it contributes no cases.
BENCHMARKS=(
	tests/test_performance.nim
)

# Written LAST, from a run (trap 4c), on 2026-09-17:
#   139 rust unit  +  10 window-registry  +  15 cross-renderer
# +  14 gui        +  31 render-integration  +  6 structural
# +   1 basic      +   1 bindings           + 23 renderer checkpoints
# = 240
#
# Was 237 on 2026-09-15. The three are RS-M14b's shutdown-flag cases in
# `rust/gpui-nim-shim/src/window.rs`
# (`test_quit_request_is_latched_and_consumed_once`,
# `test_auto_quit_deadline_round_trips_and_disarms`,
# `test_reset_windows_clears_shutdown_state`). They are in the DEFAULT
# build deliberately: the flags are what `gpui_quit` /
# `gpui_quit_after_ms` write and what the event loop's poller reads, and
# only the poller half needs a compositor.
#
# `nim: test_gui` stays at 14 because the windowed pixel suite is behind
# `when defined(gpuiBackend)` and this lane compiles without it. That
# suite's home is `just test-gui` and the `gui-tests` CI job; a count
# that moved here when the GUI lane changed would mean the two lanes had
# stopped being separable.
#
# The two `--features` runs are DELIBERATELY not in this total: they are
# conditional on `--with-gpui`, and a total that changed with a flag
# would be a fingerprint that means two different things.
EXPECTED_CASES=240

total_cases=0
failed_steps=()
declare -a REPORT

echo "=========================================================="
echo "isonim-gpui lane   (gpui-linking builds: $([ "$WITH_GPUI" -eq 1 ] && echo INCLUDED || echo 'EXCLUDED — pass --with-gpui'))"
echo "=========================================================="

# --- 0. Gates ----------------------------------------------------------
run_gate() {
	local name="$1"
	shift
	echo ""
	echo "--- $name"
	local out rc
	out="$("$@" 2>&1)"
	rc=$?
	printf '%s\n' "$out"
	[ "$rc" -eq 0 ] || failed_steps+=("$name (rc=$rc)")
	REPORT+=("$(printf '%-46s rc=%-3s cases=%s' "$name" "$rc" "n/a (gate)")")
}

run_gate "gate: GPUI pin is one version" ./tools/check_gpui_pin.sh
run_gate "gate: Rust exports == Nim bindings" ./tools/check_bindings.sh
run_gate "gate: trap-13 assertion-helper sweep" python3 tools/trap13-assertion-helper-sweep.py tests src

# --- 1. The Rust shim --------------------------------------------------
echo ""
echo "--- rust: cargo build"
cargo_out="$(cd rust && cargo build 2>&1)"
cargo_rc=$?
printf '%s\n' "$cargo_out" | tail -5
[ "$cargo_rc" -eq 0 ] || failed_steps+=("rust: cargo build (rc=$cargo_rc)")
REPORT+=("$(printf '%-46s rc=%-3s cases=%s' "rust: cargo build" "$cargo_rc" "n/a")")

echo ""
echo "--- rust: cargo test"
cargo_out="$(cd rust && cargo test 2>&1)"
cargo_rc=$?
printf '%s\n' "$cargo_out" | grep -E '^test result|^running|^error' || true
[ "$cargo_rc" -eq 0 ] || failed_steps+=("rust: cargo test (rc=$cargo_rc)")
rust_cases="$(printf '%s\n' "$cargo_out" |
	sed -n 's/^test result: ok\. \([0-9]\{1,\}\) passed.*/\1/p' |
	awk '{s+=$1} END {print s+0}')"
rust_failed="$(printf '%s\n' "$cargo_out" |
	sed -n 's/^test result: .*\. [0-9]\{1,\} passed; \([0-9]\{1,\}\) failed.*/\1/p' |
	awk '{s+=$1} END {print s+0}')"
if [ "$rust_cases" -lt 1 ]; then
	echo "REFUSING: parsed 0 Rust cases out of a run that exited $cargo_rc." >&2
	echo "The parser has stopped reading cargo's output; the total below" >&2
	echo "would be an undercount that looks like a clean lane (trap 4)." >&2
	failed_steps+=("rust: case parser read nothing")
fi
total_cases=$((total_cases + rust_cases))
REPORT+=("$(printf '%-46s rc=%-3s cases=%s (failed=%s)' "rust: cargo test" "$cargo_rc" "$rust_cases" "$rust_failed")")

if [ "$WITH_GPUI" -eq 1 ]; then
	for feat in gpui-backend gpui-headless; do
		echo ""
		echo "--- rust: cargo test --features $feat"
		cargo_out="$(cd rust && cargo test --features "$feat" 2>&1)"
		cargo_rc=$?
		printf '%s\n' "$cargo_out" | grep -E '^test result|^running|^error' || true
		[ "$cargo_rc" -eq 0 ] || failed_steps+=("rust: cargo test --features $feat (rc=$cargo_rc)")
		feat_cases="$(printf '%s\n' "$cargo_out" |
			sed -n 's/^test result: ok\. \([0-9]\{1,\}\) passed.*/\1/p' |
			awk '{s+=$1} END {print s+0}')"
		REPORT+=("$(printf '%-46s rc=%-3s cases=%s (not in total)' "rust: --features $feat" "$cargo_rc" "$feat_cases")")
	done
else
	REPORT+=("$(printf '%-46s %s' "rust: --features gpui-backend" "NOT RUN — pass --with-gpui")")
	REPORT+=("$(printf '%-46s %s' "rust: --features gpui-headless" "NOT RUN — pass --with-gpui")")
fi

# --- 2. The Nim suites -------------------------------------------------
run_nim() { # file, kind
	local file="$1" kind="$2"
	local base cache out rc cases
	base="$(basename "$file" .nim)"
	cache="$NIMCACHE_ROOT/$base"
	echo ""
	echo "--- nim: $file"
	out="$(nim c -r --hints:off --warnings:off --nimcache:"$cache" "$file" 2>&1)"
	rc=$?
	printf '%s\n' "$out" | tail -25
	[ "$rc" -eq 0 ] || failed_steps+=("nim: $file (rc=$rc)")

	case "$kind" in
	unittest)
		cases="$(printf '%s\n' "$out" | grep -c -E '^[[:space:]]*\[(OK|FAILED)\] ')"
		if [ "$rc" -eq 0 ] && [ "$cases" -lt 1 ]; then
			echo "REFUSING: $file exited 0 and produced no [OK]/[FAILED] lines." >&2
			failed_steps+=("nim: $file ran no cases")
		fi
		total_cases=$((total_cases + cases))
		REPORT+=("$(printf '%-46s rc=%-3s cases=%s' "nim: $base" "$rc" "$cases")")
		;;
	assert)
		# Not "^${base}:" — `test_basic.nim` prints its checkpoint as
		# `isonim-gpui: compile check passed`, so a basename-anchored
		# pattern read ZERO out of a suite that ran, which the lane
		# correctly refused rather than totalling as nothing.
		# Anchored to the CHECKPOINT FORM (`<prefix>: <text> passed`), not
		# to the basename: `test_basic.nim` prints its checkpoint as
		# `isonim-gpui: compile check passed`, so a `^${base}:` pattern
		# read ZERO out of a suite that had run — which the lane refused,
		# correctly, rather than totalling it as nothing. Not anchored at
		# the END either: one line reads `… passed (150 callbacks)`.
		cases="$(printf '%s\n' "$out" | grep -c -E '^[^ :]+: .*(passed|verified)')"
		if [ "$rc" -eq 0 ] && [ "$cases" -lt 1 ]; then
			echo "REFUSING: $file exited 0 and printed no checkpoints." >&2
			failed_steps+=("nim: $file printed no checkpoints")
		fi
		total_cases=$((total_cases + cases))
		REPORT+=("$(printf '%-46s rc=%-3s cases=%s (assert-style checkpoints)' "nim: $base" "$rc" "$cases")")
		;;
	bench)
		REPORT+=("$(printf '%-46s rc=%-3s cases=%s' "nim: $base" "$rc" "0 (benchmark, asserts nothing)")")
		;;
	esac
}

for f in "${UNITTEST_SUITES[@]}"; do run_nim "$f" unittest; done
for f in "${ASSERT_SCRIPTS[@]}"; do run_nim "$f" assert; done
for f in "${BENCHMARKS[@]}"; do run_nim "$f" bench; done

# --- 3. The tally ------------------------------------------------------
echo ""
echo "=========================================================="
echo "isonim-gpui lane — per-subject report"
echo "=========================================================="
printf '%s\n' "${REPORT[@]}"
echo "----------------------------------------------------------"
echo "TOTAL CASES: $total_cases   (expected $EXPECTED_CASES)"
if [ "$WITH_GPUI" -eq 0 ]; then
	echo "SUBSET RUN:  everything EXCEPT 'cargo test --features gpui-backend'"
	echo "             and '--features gpui-headless'. Those two are the only"
	echo "             steps that compile GPUI itself; the three Rust"
	echo "             integration test files are each #![cfg(feature=...)]"
	echo "             and report 'running 0 tests' without them."
fi

if [ "$total_cases" -ne "$EXPECTED_CASES" ]; then
	echo ""
	echo "CASE COUNT DRIFT: the lane ran $total_cases cases, not $EXPECTED_CASES."
	echo "A count that moves demands an explanation: either a suite stopped"
	echo "running (which does not fail — it ceases to exist) or cases were"
	echo "added. If they were added, update EXPECTED_CASES in $0 from THIS run."
	failed_steps+=("case count drift: $total_cases != $EXPECTED_CASES")
fi

echo ""
if [ "${#failed_steps[@]}" -eq 0 ]; then
	echo "LANE RESULT: OK — $total_cases cases, 0 failing steps."
	exit 0
fi
echo "LANE RESULT: FAILED — ${#failed_steps[@]} step(s):"
printf '  - %s\n' "${failed_steps[@]}"
exit 1
