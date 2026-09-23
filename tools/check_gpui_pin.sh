#!/usr/bin/env bash
# PLAT-19 — assert the GPUI pin is ONE version, everywhere.
#
# GPUI has no stable public API, so `isonim-gpui` pins it exactly. Since the
# gpui-kit alignment the pin is a crates.io version of the `gpui-pre*` family
# (a republication of Zed's own crates; see `docs/gpui-pin.md`) rather than a
# `zed-industries/zed` git revision. What the gate is for did not change: the
# pin is written in more than one place, which is trap 14's shape exactly — a
# predicate spelled twice is a predicate that will disagree with itself, and
# the disagreement here is silent, because cargo will happily resolve a
# `gpui_platform` built against one `gpui` and link it next to another.
#
# There is no way to write a cargo manifest with one literal (a version
# requirement cannot be interpolated), so the remedy is the other half of
# §14b: one gate that reads every copy and requires them to agree, plus the
# LOCK, which is the only place that records what cargo actually resolved.
#
# Checks, in order:
#   1. Every `gpui-pre*` dependency under rust/**/Cargo.toml requests the same
#      exact version, and every one of them requests a version at all.
#   2. Every resolved `gpui-pre*` package in rust/Cargo.lock is at that same
#      version — so a manifest bumped without `cargo update` is red.
#   3. NO SECOND GPUI CORE is resolved. This is the check that replaces the
#      old "one zed revision" rule and it is the one with teeth: type identity
#      in Rust is per PACKAGE, so a `gpui` from the zed git remote, or the
#      unrelated crates.io `gpui`, resolved alongside `gpui-pre` would give the
#      binary two incompatible `App`/`Entity`/`Window` types that no error
#      message calls by that name. It is also precisely the state that made
#      gpui-kit unreachable before this pin moved.
#   4. The version documented in docs/gpui-pin.md is that same string.
#
# Each has a non-vacuity floor (traps 4 / 6a): a scan that finds nothing
# satisfies "all of them agree" for free, and this gate exists precisely
# to be believed. Check 3 is the exception by construction — it asserts an
# ABSENCE, so an empty scan is the passing answer and a floor would be
# meaningless. That the FILE is read at all is inherited from checks 1 and
# 2 above it. That the check itself can fire is not inherited and is not
# argued: check 3 has TWO branches and each is planted by its own mutation
# arm — G4b writes a zed git source into the lock, G4c writes the
# crates.io `gpui` package — so both have been watched go red. Arming only
# one of two branches is how an absence check half-rots unnoticed; G4c was
# added on 2026-09-17 after exactly that was found here.
#
# Usage: ./tools/check_gpui_pin.sh
# Exit 0 = one version everywhere; 1 = disagreement; 2 = the scan failed.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RUST_DIR="$REPO_ROOT/rust"
LOCK="$RUST_DIR/Cargo.lock"
DOC="$REPO_ROOT/docs/gpui-pin.md"

# Floors. These are counts the tree has today; they are floors rather
# than equalities so adding a manifest or a transitive gpui-pre crate does
# not fail here, and they are non-zero so an empty scan cannot pass.
MIN_MANIFEST_PINS=2
MIN_LOCK_PINS=4

# NAMED BY PREFIX, VERSIONED ON THEIR OWN. `gpui-pre-reqwest` is the
# republication of zed's `reqwest` fork (its description: "higher level HTTP
# client library zed-industries/request@…"), published at reqwest's own
# version line (0.12.x), not in lockstep with the GPUI core. It entered the
# lock with gpui-kit (2026-09-17) and the family scan below — which matches
# the `gpui-pre` PREFIX — read it as a second resolved gpui-pre version and
# went red on every run after. It is excluded BY NAME rather than by
# loosening the prefix match, so a genuinely new `gpui-pre-*` crate still
# has to be at the pin; and section 3 still refuses any package, this one
# included, that provides a second GPUI core.
INDEPENDENTLY_VERSIONED=("gpui-pre-reqwest")

fail() {
	echo "check_gpui_pin: $*" >&2
	exit 1
}
refuse() {
	echo "check_gpui_pin: REFUSING TO REPORT — $*" >&2
	exit 2
}

[ -d "$RUST_DIR" ] || refuse "no rust/ directory at $RUST_DIR"
[ -f "$LOCK" ] || refuse "no Cargo.lock at $LOCK"

# --- 1. the manifests ---------------------------------------------------
manifests=()
while IFS= read -r m; do manifests+=("$m"); done < <(
	find "$RUST_DIR" -name Cargo.toml -not -path '*/target/*' | sort
)
[ "${#manifests[@]}" -gt 0 ] || refuse "found no Cargo.toml under rust/"

# The dependency KEYS are `gpui` / `gpui_platform`; the PACKAGES are
# `gpui-pre` / `gpui-pre-platform`. Anchor on the package, because that is
# what decides type identity and what the lock records.
dep_lines="$(grep -hE 'package[[:space:]]*=[[:space:]]*"gpui-pre' "${manifests[@]}" || true)"
dep_count="$(printf '%s\n' "$dep_lines" | grep -c . || true)"
if [ "$dep_count" -lt "$MIN_MANIFEST_PINS" ]; then
	refuse "found $dep_count gpui-pre dependencies in rust/**/Cargo.toml, floor is $MIN_MANIFEST_PINS"
fi

manifest_revs="$(printf '%s\n' "$dep_lines" |
	grep -oE 'version[[:space:]]*=[[:space:]]*"=[0-9]+\.[0-9]+\.[0-9]+"' |
	grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | sort)"
manifest_count="$(printf '%s\n' "$manifest_revs" | grep -c . || true)"
# Equality, not a floor: every gpui-pre dependency must carry an EXACT
# version. A caret requirement, or none at all, parses to nothing here and
# would otherwise be dropped silently out of a set the next line then finds
# unanimous — §4 in one line.
if [ "$manifest_count" -ne "$dep_count" ]; then
	fail "$dep_count gpui-pre dependencies but only $manifest_count exact (\"=X.Y.Z\") version requirements — every one must be pinned exactly"
fi

distinct_manifest="$(printf '%s\n' "$manifest_revs" | sort -u)"
distinct_manifest_count="$(printf '%s\n' "$distinct_manifest" | grep -c . || true)"
if [ "$distinct_manifest_count" -ne 1 ]; then
	echo "Pinned versions disagree across rust/**/Cargo.toml:" >&2
	mapfile -t manifest_list <<<"$distinct_manifest"
	printf '  %s\n' "${manifest_list[@]}" >&2
	fail "expected exactly one version, found $distinct_manifest_count"
fi
PIN="$distinct_manifest"
echo "manifests: $manifest_count pin(s), all at $PIN"

# --- 2. the lock --------------------------------------------------------
# Every `[[package]]` whose name is in the gpui-pre family, and the version
# recorded on the line after it.
lock_pairs="$(awk -v skip="${INDEPENDENTLY_VERSIONED[*]}" '
	BEGIN { n = split(skip, s, " "); for (i = 1; i <= n; i++) excluded["\"" s[i] "\""] = 1 }
	/^name = "gpui-pre/ { want = !($3 in excluded); next }
	want && /^version = / { gsub(/[">]/, "", $3); print $3; want = 0 }
' "$LOCK")"
lock_total="$(printf '%s\n' "$lock_pairs" | grep -c . || true)"
if [ "$lock_total" -lt "$MIN_LOCK_PINS" ]; then
	refuse "found $lock_total gpui-pre packages in Cargo.lock, floor is $MIN_LOCK_PINS"
fi
lock_revs="$(printf '%s\n' "$lock_pairs" | sort -u)"
lock_distinct_count="$(printf '%s\n' "$lock_revs" | grep -c . || true)"
if [ "$lock_distinct_count" -ne 1 ]; then
	echo "Cargo.lock resolved MORE THAN ONE gpui-pre version:" >&2
	mapfile -t lock_list <<<"$lock_revs"
	printf '  %s\n' "${lock_list[@]}" >&2
	fail "expected exactly one resolved version, found $lock_distinct_count"
fi
if [ "$lock_revs" != "$PIN" ]; then
	fail "manifest pins $PIN but Cargo.lock resolved $lock_revs — run 'cargo update'"
fi
echo "lock:      $lock_total gpui-pre package(s), all at $PIN"

# --- 3. no second GPUI core --------------------------------------------
# `gpui-pre` is THE gpui core in this graph. Anything else that provides one
# is a second set of `App` / `Entity<T>` / `Window` types.
rogue=""
if grep -qE '^source = "git\+https://github.com/zed-industries/zed' "$LOCK"; then
	rogue="$rogue\n  a zed-industries/zed git source"
fi
if grep -qE '^name = "gpui"$' "$LOCK"; then
	rogue="$rogue\n  the crates.io \`gpui\` package"
fi
if [ -n "$rogue" ]; then
	echo "Cargo.lock resolved a SECOND GPUI core alongside gpui-pre:" >&2
	printf '%b\n' "$rogue" >&2
	echo "Rust type identity is per package, so the two do not unify and" >&2
	echo "gpui-kit cannot be handed anything this shim builds." >&2
	fail "expected gpui-pre to be the only GPUI core in the graph"
fi
echo "cores:     gpui-pre is the only GPUI core in the lock"

# --- 4. the documented procedure ---------------------------------------
[ -f "$DOC" ] || refuse "no pin document at $DOC"
# THE HEADLINE, not "anywhere in the file". Asking whether the pin occurs
# somewhere in the document is satisfied by the `sed` command in its own
# update procedure, which names the OLD version by construction — so the
# check passed over a headline that had been changed to a different
# revision. Found by arm G5 on 2026-09-15, which reported rc=0 over exactly
# that plant; §4d's shape, where the prose ABOUT the thing satisfies the
# scan for the thing.
#
# POSIX ERE only: `\b` is a GNU extension and this gate runs on the macOS
# runners as well (trap 4).
doc_headline="$(grep -E '^\*\*Pinned version:\*\*' "$DOC" || true)"
if [ -z "$doc_headline" ]; then
	refuse "docs/gpui-pin.md has no '**Pinned version:**' headline to read"
fi
doc_revs="$(printf '%s\n' "$doc_headline" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | sort -u)"
doc_count="$(printf '%s\n' "$doc_revs" | grep -c . || true)"
if [ "$doc_count" -ne 1 ]; then
	refuse "the '**Pinned version:**' headline names $doc_count versions, expected 1"
fi
if [ "$doc_revs" != "$PIN" ]; then
	fail "the documented version is not $PIN — the headline names $doc_revs"
fi
echo "docs:      docs/gpui-pin.md headline names $PIN"

echo ""
echo "GPUI pin OK: gpui-pre $PIN"
