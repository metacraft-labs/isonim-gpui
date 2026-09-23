#!/usr/bin/env bash
# PLAT-42 — build each (candidate x surface) cell of the editor decision on its
# own and report its verdict. Re-runnable: the losers are a command, not prose.
#
# Output: one JSON object per line on stdout —
#   {"cell":"b-values","compiles":false,"private":[...],"missing":[...]}
# `private` lists items rustc refused as private (the fork bill, K3);
# `missing` lists items that do not exist at all (no fork exposes those).
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
cd "$here/../rust" || exit 1
for cell in b-pointer b-status b-values b-flow b-foldaware a-pointer a-status a-values a-flow; do
	log="$(cargo build -p gpui-kit-probe --no-default-features --features "$cell" --message-format short 2>&1)"
	rc=$?
	# The parser is its own file: stdin carries the LOG. (An earlier draft gave
	# python3 a heredoc AND a here-string; the here-string won stdin and Python
	# tried to execute the cargo log as its program.)
	printf '%s\n' "$log" | python3 "$here/plat42_cell_verdict.py" "$cell" "$rc"
done
