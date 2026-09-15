#!/usr/bin/env bash
# Validate that all Rust extern "C" functions have corresponding Nim bindings.
#
# Usage: ./tools/check_bindings.sh
#
# Exits 0 if all exports are covered, 1 if there are mismatches,
# 2 if the scan itself could not read the tree.
#
# PLAT-19 repaired three things here, all of which were making it report
# something other than what it claims:
#
#   1. THE SUBJECT WAS ONE FILE. It scanned `src/lib.rs` only, while six
#      exports live in `src/gpui_headless.rs`. Those six were reported as
#      "EXTRA in Nim bindings" — a red run, at HEAD, for as long as the
#      headless path had existed. Trap 6: the subject of a scan is a
#      claim, and "all N Rust exports" was claiming lib.rs was the whole
#      population. The scan now covers `src/*.rs`.
#
#   2. `pub unsafe extern "C" fn` DID NOT MATCH. `gpui_free_pixels` is
#      spelled that way and was invisible to a pattern that required
#      `pub extern`. One symbol, and it is the sixth of the six.
#
#   3. THERE WAS NO NON-VACUITY FLOOR (traps 4 / 6a). The mechanism is
#      `RUST_COUNT=$(echo "$RUST_FUNCS" | wc -l)`, which answers **1**
#      for the empty string, and a `comm` over two empty sets, which
#      reports no MISSING and no EXTRA. Put those together and the
#      script prints "All 1 Rust exports have matching Nim bindings"
#      and exits 0 over a scan that read nothing.
#
#      AN EARLIER DRAFT OF THIS COMMENT CLAIMED THAT TRANSCRIPT WAS
#      PRODUCED BY POINTING `RUST_SRC` AT A NONEXISTENT FILE. IT IS NOT.
#      The measurements, run 2026-09-15 against a copy of the HEAD
#      script with `REPO_ROOT` pinned to this repository:
#
#        # one side missing — `set -euo pipefail` (line 8) aborts on the
#        # failing grep before anything is printed
#        $ sed 's|src/lib.rs|src/NOPE.rs|' head.sh > a.sh && ./a.sh
#        grep: …/rust/gpui-nim-shim/src/NOPE.rs: No such file or directory
#        EXIT=2
#
#        # one side missing, errexit removed — the OTHER side still has
#        # 59 names, so all 59 are EXTRA
#        EXIT=1
#
#        # BOTH sides empty AND errexit removed — the vacuous pass
#        $ sed -e 's|src/lib.rs|src/NOPE.rs|' \
#              -e 's|src/isonim_gpui/bindings.nim|src/NOPE.nim|' \
#              -e '/^set -euo pipefail$/d' head.sh > c.sh && ./c.sh
#        Rust extern "C" exports: 1
#        Nim binding imports:     1
#
#        All 1 Rust exports have matching Nim bindings.
#        EXIT=0
#
#      So `set -euo pipefail` was load-bearing, and the honest statement
#      of why the floor exists is not "the script passed over a missing
#      file" — it is that `errexit` reports EVERY scan failure as a bare
#      `1` with no output, which is the same number a real mismatch
#      exits with (§5: a sentinel that collides with a legitimate
#      value). "The scan could not read the tree" and "the bindings
#      disagree" are different facts and the caller could not tell them
#      apart.
#
#      That is also why the two scan assignments below are written to
#      NOT abort: with them aborting, the floor could never be reached
#      and its named refusal could never print. Measured before the
#      change, with the current script pointed at a directory whose
#      single `.rs` file declares no exports: `EXIT=1`, no output, the
#      REFUSING line never reached. A floor behind an `errexit` is not
#      a floor (§6a).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RUST_SRC_DIR="$REPO_ROOT/rust/gpui-nim-shim/src"
NIM_BINDINGS="$REPO_ROOT/src/isonim_gpui/bindings.nim"

# A floor, not a formality. These are lower bounds well under the current
# figures (59 / 59 on 2026-09-15); they exist so a scan that has stopped
# reading the tree cannot report a clean sweep, and they are deliberately
# NOT the exact counts, which would turn every added export into a failure
# in this file. The exact agreement is what the comm below asserts.
MIN_EXPORTS=40
MIN_BINDINGS=40

if [ ! -d "$RUST_SRC_DIR" ]; then
	echo "REFUSING TO REPORT: Rust source directory not found: $RUST_SRC_DIR" >&2
	exit 2
fi
if [ ! -f "$NIM_BINDINGS" ]; then
	echo "REFUSING TO REPORT: Nim bindings not found: $NIM_BINDINGS" >&2
	exit 2
fi

# Extract exported function names from every Rust source file in the crate.
# `pub extern "C" fn` and `pub unsafe extern "C" fn` are both exports.
# `|| true` is deliberate and is the subject of note 3 above: a `grep`
# that matches nothing exits 1, `pipefail` propagates it, and `errexit`
# would then kill the script HERE — before the floor below has a chance
# to say which scan came back empty, and with the same exit code a real
# mismatch uses. Let the assignment succeed with an empty set; the floor
# is what decides, and it decides out loud.
RUST_FUNCS=$(grep -rhoP 'pub (?:unsafe )?extern "C" fn \K\w+' "$RUST_SRC_DIR"/*.rs | sort -u || true)
RUST_COUNT=$(printf '%s\n' "$RUST_FUNCS" | grep -c . || true)

# Extract imported function names from the Nim bindings.
NIM_FUNCS=$(grep -oP '(?<=proc )\w+(?=\*)' "$NIM_BINDINGS" | sort -u || true)
NIM_COUNT=$(printf '%s\n' "$NIM_FUNCS" | grep -c . || true)

echo "Rust extern \"C\" exports: $RUST_COUNT  (scanned $(find "$RUST_SRC_DIR" -maxdepth 1 -name "*.rs" | wc -l) files)"
echo "Nim binding imports:     $NIM_COUNT"
echo ""

if [ "$RUST_COUNT" -lt "$MIN_EXPORTS" ]; then
	echo "REFUSING TO REPORT: found $RUST_COUNT Rust exports, floor is $MIN_EXPORTS." >&2
	echo "The scan is not reading the crate; a clean sweep here would be vacuous." >&2
	exit 2
fi
if [ "$NIM_COUNT" -lt "$MIN_BINDINGS" ]; then
	echo "REFUSING TO REPORT: found $NIM_COUNT Nim bindings, floor is $MIN_BINDINGS." >&2
	exit 2
fi

MISSING=$(comm -23 <(printf '%s\n' "$RUST_FUNCS") <(printf '%s\n' "$NIM_FUNCS"))
EXTRA=$(comm -13 <(printf '%s\n' "$RUST_FUNCS") <(printf '%s\n' "$NIM_FUNCS"))

STATUS=0

if [ -n "$MISSING" ]; then
	echo "MISSING in Nim bindings (present in Rust but not in Nim):"
	mapfile -t missing_list <<<"$MISSING"
	printf '  - %s\n' "${missing_list[@]}"
	STATUS=1
fi

if [ -n "$EXTRA" ]; then
	echo "EXTRA in Nim bindings (present in Nim but not in Rust):"
	mapfile -t extra_list <<<"$EXTRA"
	printf '  - %s\n' "${extra_list[@]}"
	STATUS=1
fi

if [ "$STATUS" -eq 0 ]; then
	echo "All $RUST_COUNT Rust exports have matching Nim bindings."

	# Also check the generated bindings file if it exists
	GENERATED="$REPO_ROOT/src/isonim_gpui/bindings_generated.nim"
	if [ -f "$GENERATED" ]; then
		GEN_FUNCS=$(grep -oP 'importc: "\K\w+' "$GENERATED" | sort -u)
		GEN_MISSING=$(comm -23 <(printf '%s\n' "$RUST_FUNCS") <(printf '%s\n' "$GEN_FUNCS"))
		if [ -n "$GEN_MISSING" ]; then
			echo ""
			echo "WARNING: bindings_generated.nim is missing:"
			mapfile -t gen_missing_list <<<"$GEN_MISSING"
			printf '  - %s\n' "${gen_missing_list[@]}"
			echo "Run 'just generate-bindings' to regenerate."
			STATUS=1
		else
			echo "Generated bindings also match ($NIM_COUNT/$RUST_COUNT)."
		fi
	fi
fi

echo ""
exit $STATUS
