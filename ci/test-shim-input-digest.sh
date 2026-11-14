#!/usr/bin/env bash
# Exercise the real digest in a real git repository: every state a shim's
# inputs can be in must move it, an edit outside the inputs must not, and a
# directory that is not a checkout must be refused. The stamp writer is run
# against the same repository, with the real `rustc` on PATH. No mock is used.
set -euo pipefail
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/gpui-shim-digest.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

copy="$scratch/copy"
mkdir -p "$copy/tools" "$copy/rust/gpui-nim-shim/src"
cp "$repo/tools/shim_input_digest.sh" "$repo/tools/write_shim_stamp.sh" "$copy/tools/"
cd "$copy"
printf 'rust/target\n' >.gitignore
for f in rust/Cargo.toml rust/Cargo.lock rust/gpui-nim-shim/Cargo.toml \
	rust/gpui-nim-shim/src/lib.rs flake.nix flake.lock Justfile README.md; do
	printf 'initial %s\n' "$f" >"$f"
done
git init -q
git add -A
git -c user.name=test -c user.email=test@example.invalid -c commit.gpgsign=false \
	commit -qm fixture

digest() { bash tools/shim_input_digest.sh "$copy"; }
baseline="$(digest)"
case "$baseline" in
[0-9a-f][0-9a-f][0-9a-f]*) [ "${#baseline}" -eq 64 ] || fail "not a sha256: $baseline" ;;
*) fail "not a sha256: $baseline" ;;
esac
[ "$(digest)" = "$baseline" ] || fail 'two runs over one tree disagree'
echo 'PASS: stable across runs'

# Each change must move the digest, and undoing it must move it back: the
# second half is what proves that the first was this change and not noise.
assert_moves() {
	local label="$1" after
	after="$(digest)"
	[ "$after" != "$baseline" ] || fail "$label did not move the digest"
	git reset -q --hard
	git clean -qfd
	[ "$(digest)" = "$baseline" ] || fail "undoing $label did not restore the digest"
	echo "PASS: $label moves the digest"
}
printf 'edit\n' >>rust/gpui-nim-shim/src/lib.rs
assert_moves 'an unstaged edit'
printf 'edit\n' >>rust/gpui-nim-shim/src/lib.rs
git add rust/gpui-nim-shim/src/lib.rs
assert_moves 'a staged edit'
printf 'new\n' >rust/gpui-nim-shim/src/added.rs
assert_moves 'an untracked file'
rm rust/gpui-nim-shim/src/lib.rs
assert_moves 'a deleted file'
for root in rust/Cargo.lock rust/Cargo.toml flake.lock flake.nix Justfile; do
	printf 'edit\n' >>"$root"
	assert_moves "an edit to $root"
done

# Index state is not an input. A shim built from a modified tree has to stay
# fresh when that same content is staged and committed, or every landing would
# refuse the shim that was built from it.
printf 'edit\n' >>rust/gpui-nim-shim/src/lib.rs
printf 'new\n' >rust/gpui-nim-shim/src/added.rs
rm rust/gpui-nim-shim/Cargo.toml
modified="$(digest)"
[ "$modified" != "$baseline" ] || fail 'the modified tree did not move the digest'
git add -A
[ "$(digest)" = "$modified" ] || fail 'staging moved the digest'
git -c user.name=test -c user.email=test@example.invalid -c commit.gpgsign=false \
	commit -qm modified
[ "$(digest)" = "$modified" ] || fail 'committing moved the digest'
git reset -q --hard HEAD~1
[ "$(digest)" = "$baseline" ] || fail 'undoing the commit did not restore the digest'
echo 'PASS: staging and committing the same content does not move it'

printf 'edit\n' >>README.md
[ "$(digest)" = "$baseline" ] || fail 'an edit outside the roots moved the digest'
git checkout -q README.md
mkdir -p rust/target/debug
printf 'build output\n' >rust/target/debug/output
[ "$(digest)" = "$baseline" ] || fail 'ignored build output moved the digest'
echo 'PASS: an edit outside the inputs does not move it'

# A directory inside this checkout is the dangerous shape: `git rev-parse`
# answers for it, and untracked files there would be digested as if they were
# the shim's. The file is what makes the root check the only thing refusing it.
mkdir nested
printf 'not this checkout\n' >nested/Justfile
for target in "$scratch" "$copy/nested"; do
	rc=0
	answer="$(bash "$copy/tools/shim_input_digest.sh" "$target" 2>/dev/null)" || rc=$?
	[ "$rc" -ne 0 ] || fail "$target was digested"
	[ -z "$answer" ] || fail "a refusal printed a digest: $answer"
done
rm -r nested
echo 'PASS: outside a checkout root is refused, with nothing on stdout'

printf 'shim bytes\n' >"$scratch/shim"
bash tools/write_shim_stamp.sh "$scratch/stamp" "$scratch/shim" "$baseline" windowed
grep -qx "input_digest=$baseline" "$scratch/stamp" || fail 'the stamp lost the digest'
grep -qx 'artefact_bytes=11' "$scratch/stamp" || fail 'the stamp lost the size'
grep -qx "built_by=$(rustc -V)" "$scratch/stamp" || fail 'the stamp lost the compiler'
if bash tools/write_shim_stamp.sh "$scratch/s" "$scratch/shim" "" windowed 2>/dev/null; then
	fail 'an empty digest was stamped'
fi
if bash tools/write_shim_stamp.sh "$scratch/s" "$scratch/none" "$baseline" windowed 2>/dev/null; then
	fail 'a missing artefact was stamped'
fi
echo 'PASS: the stamp records its fields and refuses an empty digest or artefact'
