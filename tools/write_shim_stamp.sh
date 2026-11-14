#!/usr/bin/env bash
# Write the provenance stamp of one staged shim.
#
# Usage: tools/write_shim_stamp.sh <stamp> <artefact> <input-digest> <kind>
#
# The fields and their spelling are the ones `bf_write_stamp` writes in
# codetracer's `ci/lib/build-freshness.sh`, whose `bf_assert_fresh` is what
# reads this file: it refuses a stamp that does not describe the bytes beside
# it, and one whose input digest differs from `tools/shim_input_digest.sh` run
# against the tree now. `artefact_sha256` is what catches a shim copied in from
# elsewhere with an mtime that says nothing.
set -euo pipefail

stamp="$1"
artefact="$2"
input_digest="$3"
kind="$4"
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [ ! -f "$artefact" ]; then
	echo "write_shim_stamp: cannot stamp, $artefact does not exist" >&2
	exit 1
fi
if [ -z "$input_digest" ]; then
	echo "write_shim_stamp: refusing to write an empty input digest to $stamp" >&2
	exit 1
fi
if command -v sha256sum >/dev/null; then
	artefact_sha256="$(sha256sum "$artefact" | cut -d' ' -f1)"
else
	artefact_sha256="$(shasum -a 256 "$artefact" | cut -d' ' -f1)"
fi

# SHIM_BUILT_BY names another builder, for a shim that rustc did not build.
built_by="${SHIM_BUILT_BY:-$(rustc -V)}"

{
	echo "# Written by \`just plat37-shims\` for ${artefact##*/}. DO NOT EDIT BY HAND:"
	echo "# the only way to make this file true again is to rebuild the shim."
	echo "kind=$kind"
	echo "artefact=$artefact"
	echo "artefact_sha256=$artefact_sha256"
	echo "artefact_bytes=$(wc -c <"$artefact" | tr -d ' ')"
	echo "input_digest=$input_digest"
	echo "built_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
	echo "built_by=$built_by"
	printf 'repo_head=%s %s %s\n' "$(basename "$repo")" \
		"$(git -C "$repo" rev-parse HEAD)" "$(git -C "$repo" rev-parse --abbrev-ref HEAD)"
} >"$stamp"
