#!/usr/bin/env bash
# PLAT-19 — assert the GPUI pin is ONE revision, everywhere.
#
# GPUI has no stable public API and no crates.io release we can use, so
# `isonim-gpui` pins `zed-industries/zed` by commit. That pin is written
# in more than one place, which is trap 14's shape exactly — a predicate
# spelled twice is a predicate that will disagree with itself, and the
# disagreement here is silent: Cargo will happily resolve TWO checkouts
# of Zed at two revisions and link a `gpui_platform` from one against a
# `gpui` from the other.
#
# There is no way to write a cargo manifest with one literal (a git `rev`
# cannot be interpolated), so the remedy is the other half of §14b: one
# gate that reads every copy and requires them to agree, plus the LOCK,
# which is the only place that records what cargo actually resolved.
#
# Checks, in order:
#   1. Every `rev = "…"` under rust/**/Cargo.toml is the same string.
#   2. Every resolved `zed-industries/zed?rev=` in rust/Cargo.lock is that
#      same string — so a manifest bumped without `cargo update` is red.
#   3. The revision documented in docs/gpui-pin.md is that same string.
#
# Each has a non-vacuity floor (traps 4 / 6a): a scan that finds nothing
# satisfies "all of them agree" for free, and this gate exists precisely
# to be believed.
#
# Usage: ./tools/check_gpui_pin.sh
# Exit 0 = one revision everywhere; 1 = disagreement; 2 = the scan failed.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RUST_DIR="$REPO_ROOT/rust"
LOCK="$RUST_DIR/Cargo.lock"
DOC="$REPO_ROOT/docs/gpui-pin.md"

# Floors. These are counts the tree has today; they are floors rather
# than equalities so adding a manifest or a transitive zed crate does not
# fail here, and they are non-zero so an empty scan cannot pass.
MIN_MANIFEST_PINS=2
MIN_LOCK_PINS=4

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

manifest_revs="$(grep -hoE 'rev[[:space:]]*=[[:space:]]*"[0-9a-f]{7,40}"' "${manifests[@]}" |
	grep -oE '[0-9a-f]{7,40}' | sort)"
manifest_count="$(printf '%s\n' "$manifest_revs" | grep -c . || true)"
if [ "$manifest_count" -lt "$MIN_MANIFEST_PINS" ]; then
	refuse "found $manifest_count pinned revisions in rust/**/Cargo.toml, floor is $MIN_MANIFEST_PINS"
fi

distinct_manifest="$(printf '%s\n' "$manifest_revs" | sort -u)"
distinct_manifest_count="$(printf '%s\n' "$distinct_manifest" | grep -c . || true)"
if [ "$distinct_manifest_count" -ne 1 ]; then
	echo "Pinned revisions disagree across rust/**/Cargo.toml:" >&2
	mapfile -t manifest_list <<<"$distinct_manifest"
	printf '  %s\n' "${manifest_list[@]}" >&2
	fail "expected exactly one revision, found $distinct_manifest_count"
fi
PIN="$distinct_manifest"
echo "manifests: $manifest_count pin(s), all at $PIN"

# --- 2. the lock --------------------------------------------------------
lock_revs="$(grep -oE 'zed-industries/zed\?rev=[0-9a-f]{40}' "$LOCK" |
	grep -oE '[0-9a-f]{40}' | sort -u)"
lock_total="$(grep -c 'zed-industries/zed?rev=' "$LOCK" || true)"
if [ "$lock_total" -lt "$MIN_LOCK_PINS" ]; then
	refuse "found $lock_total zed git sources in Cargo.lock, floor is $MIN_LOCK_PINS"
fi
lock_distinct_count="$(printf '%s\n' "$lock_revs" | grep -c . || true)"
if [ "$lock_distinct_count" -ne 1 ]; then
	echo "Cargo.lock resolved MORE THAN ONE zed revision:" >&2
	mapfile -t lock_list <<<"$lock_revs"
	printf '  %s\n' "${lock_list[@]}" >&2
	fail "expected exactly one resolved revision, found $lock_distinct_count"
fi
if [ "$lock_revs" != "$PIN" ]; then
	fail "manifest pins $PIN but Cargo.lock resolved $lock_revs — run 'cargo update -p gpui'"
fi
echo "lock:      $lock_total zed source line(s), all at $PIN"

# --- 3. the documented procedure ---------------------------------------
[ -f "$DOC" ] || refuse "no pin document at $DOC"
# THE HEADLINE, not "anywhere in the file". Asking whether the pin occurs
# somewhere in the document is satisfied by the `sed` command in its own
# update procedure, which names the OLD revision by construction — so the
# check passed over a headline that had been changed to a different
# commit. Found by arm G5 on 2026-09-15, which reported rc=0 over exactly
# that plant; §4d's shape, where the prose ABOUT the thing satisfies the
# scan for the thing.
#
# POSIX ERE only: `\b` is a GNU extension and this gate runs on the macOS
# runners as well (trap 4).
doc_headline="$(grep -E '^\*\*Pinned revision:\*\*' "$DOC" || true)"
if [ -z "$doc_headline" ]; then
	refuse "docs/gpui-pin.md has no '**Pinned revision:**' headline to read"
fi
doc_revs="$(printf '%s\n' "$doc_headline" | grep -oE '[0-9a-f]{40}' | sort -u)"
doc_count="$(printf '%s\n' "$doc_revs" | grep -c . || true)"
if [ "$doc_count" -ne 1 ]; then
	refuse "the '**Pinned revision:**' headline names $doc_count 40-hex revisions, expected 1"
fi
if [ "$doc_revs" != "$PIN" ]; then
	fail "the documented revision is not $PIN — the headline names $doc_revs"
fi
echo "docs:      docs/gpui-pin.md headline names $PIN"

echo ""
echo "GPUI pin OK: zed-industries/zed @ $PIN"
