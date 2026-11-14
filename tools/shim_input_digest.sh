#!/usr/bin/env bash
# Print the input identity of the Rust shim as ONE sha256.
#
# Usage: tools/shim_input_digest.sh [repo-root]
#
# Exits 0 and prints the digest on stdout, or exits non-zero with NOTHING on
# stdout, so that `digest="$(…)"` can never hand a caller an empty string that
# compares equal to the next empty string.
#
# ## Why this is one script, called from two repositories
#
# `just plat37-shims` records this digest in the stamp beside each staged shim,
# and a consumer that is about to load a staged shim recomputes it from the
# tree it stands in. Those two sides are a build and the check on that build,
# so they run this same file: two copies could drift apart and go on agreeing
# with themselves. It needs only git and a sha256 tool, because the consumer's
# dev shell has no reason to carry cargo, and a shim build from there does not
# link at all.
#
# ## Why the tree and not `cargo`'s dep-info
#
# Nothing here asks which files the compiler read; the roots are a deliberate
# OVER-approximation, so an edit that cannot change the shim still reads as
# stale. That is the safe direction, and the remedy (rebuilding) always clears
# it. `Justfile` is a root because it holds the feature flags that select the
# build, and `flake.nix`/`flake.lock` because they pin rustc. Crates fetched
# through cargo are identified by `rust/Cargo.lock`.
#
# ## Why the working tree and never the index
#
# The compiler reads the working tree, so that is what this hashes: every file
# git tracks or would track under the roots, by content, with the path beside
# it. A file that is gone contributes no line, and its absence moves the digest.
# Nothing here reads a blob id out of the index, because index state is not an
# input: if `git add` or `git commit` moved this digest, a shim built from a
# modified tree would read as stale the moment that tree was committed.
set -euo pipefail

repo="$(cd "${1:-$(dirname "${BASH_SOURCE[0]}")/..}" && pwd -P)"
roots=(rust/Cargo.toml rust/Cargo.lock rust/gpui-nim-shim flake.nix flake.lock Justfile)

# A copy nested inside some other repository (this workspace's root is one)
# would answer `rev-parse` too, and then list none of these paths.
toplevel="$(git -C "$repo" rev-parse --show-toplevel 2>/dev/null)" || toplevel=""
if [ -z "$toplevel" ] || [ "$(cd "$toplevel" && pwd -P)" != "$repo" ]; then
	echo "shim_input_digest: $repo is not the root of a git checkout, so the" >&2
	echo "  shim's inputs cannot be listed. Refusing rather than digesting nothing." >&2
	exit 2
fi

if command -v sha256sum >/dev/null; then
	sha256() { sha256sum; }
elif command -v shasum >/dev/null; then
	sha256() { shasum -a 256; }
else
	echo "shim_input_digest: neither sha256sum nor shasum is on PATH" >&2
	exit 2
fi

# Reads NUL-separated paths and prints `<blob> <path>` for each file that exists.
file_lines() {
	local f blob
	while IFS= read -r -d '' f; do
		blob="$(git -C "$repo" hash-object -- "$repo/$f" 2>/dev/null)" || continue
		printf '%s %s\n' "$blob" "$f"
	done
}

# `sort -u`: an unmerged path is listed once per stage.
lines="$(
	git -C "$repo" ls-files -z --cached --others --exclude-standard -- "${roots[@]}" |
		file_lines | LC_ALL=C sort -u
)"
if [ -z "$lines" ]; then
	echo "shim_input_digest: $repo has no file under: ${roots[*]}" >&2
	exit 2
fi
printf '%s\n' "$lines" | sha256 | cut -d' ' -f1
