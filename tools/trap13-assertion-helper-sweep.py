#!/usr/bin/env python3
"""Trap-13 sweep for isonim-gpui's Nim suites.

Trap 13 (codetracer-specs/Testing/Verification-Harness-Traps.md): a
``unittest.check`` written inside a plain ``proc`` resolves to
``unittest``'s module-level ``testStatusIMPL`` fallback instead of the
running test's local, so the failed comparison is PRINTED and the case
still reports ``[OK]``.

Trap 13a says the naive grep is wrong in two opposite directions, so
this sweep implements the shape 13a prescribes:

  * a PER-FILE fixed point, seeded with ``check`` / ``require`` /
    ``expect`` / ``fail``, growing by any ``template`` in the same file
    whose body mentions a name already in the set, iterated to
    saturation — because ``proc p() = ck a == b`` where ``ck`` is an
    asserting template is the shape that actually recurs, and a grep
    for ``check`` finds the template (which is correct) and not the
    proc (which is the defect);
  * propagation scoped to ONE FILE, because these suites do not import
    one another and a merged namespace invents findings;
  * ``.``-qualified tokens excluded, so ``Foo.expect(...)`` on a type of
    our own is not reported;
  * comments dropped and string literals blanked before any matching,
    so prose about the trap is not reported as the trap (13a / 4d);
  * one-line routines read as "the text after the first top-level ``=``".

And, per trap 4 / 13a's closing paragraph, the sweep runs a PLANTED
CONTROL on every invocation and refuses to report when the control does
not come out exactly as expected. A scanner that has stopped reading Nim
must not be able to produce a clean sweep.

Usage:  tools/trap13-assertion-helper-sweep.py [paths...]
Exit 0 = control passed and no findings; 1 = findings; 2 = control failed.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

SEEDS = {"check", "require", "expect", "fail"}

ROUTINE_RE = re.compile(r"^(\s*)(proc|func|method|iterator)\s+([A-Za-z_][A-Za-z0-9_]*)\b")
TEMPLATE_RE = re.compile(r"^(\s*)template\s+([A-Za-z_][A-Za-z0-9_]*)\b")
# A bare identifier NOT preceded by a dot (13a: exclude `.`-qualified tokens).
IDENT_RE = re.compile(r"(?<![.\w])([A-Za-z_][A-Za-z0-9_]*)")


def strip_noise(line: str) -> str:
    """Blank string/char literals and drop comments (13a's normalisations)."""
    out = []
    i = 0
    n = len(line)
    while i < n:
        c = line[i]
        if c == '"':
            out.append('""')
            i += 1
            while i < n:
                if line[i] == "\\":
                    i += 2
                    continue
                if line[i] == '"':
                    i += 1
                    break
                i += 1
            continue
        if c == "'":
            out.append("''")
            i += 1
            while i < n:
                if line[i] == "\\":
                    i += 2
                    continue
                if line[i] == "'":
                    i += 1
                    break
                i += 1
            continue
        if c == "#":
            break
        out.append(c)
        i += 1
    return "".join(out)


def routine_blocks(lines: list[str], regex: re.Pattern):
    """Yield (name, start_line_1based, body_text) for each routine."""
    for idx, raw in enumerate(lines):
        line = strip_noise(raw)
        m = regex.match(line)
        if not m:
            continue
        indent = len(m.group(1))
        name = m.group(len(m.groups()))
        eq = line.find("=")
        inline = line[eq + 1 :] if eq != -1 else ""
        body = [inline]
        j = idx + 1
        while j < len(lines):
            nxt = strip_noise(lines[j])
            if nxt.strip() == "":
                j += 1
                continue
            nxt_indent = len(nxt) - len(nxt.lstrip())
            if nxt_indent <= indent:
                break
            body.append(nxt)
            j += 1
        yield name, idx + 1, "\n".join(body)


def idents(text: str) -> set[str]:
    return set(IDENT_RE.findall(text))


# `var ck = ...` / `let ck: T = ...` / `const ck = ...` — a local binding
# SHADOWS a same-named template for the rest of the routine, so a body
# mentioning it is not reaching the template. This is 13a's five false
# positives (a local sharing a template's name) removed by Nim's own scoping
# rule rather than by a heuristic. 13a scoped the fixed point per file and
# that is necessary but not sufficient: the collision can be WITHIN one file,
# which is exactly what the planted control's
# `localVariableNamedLikeTemplate` exercises.
LOCAL_DECL_RE = re.compile(r"(?<![.\w])(?:var|let|const)\s+([A-Za-z_][A-Za-z0-9_]*)")


def shadowed(text: str) -> set[str]:
    return set(LOCAL_DECL_RE.findall(text))


def sweep_file(path: Path) -> list[tuple[str, int, str]]:
    lines = path.read_text(encoding="utf-8", errors="replace").splitlines()

    # Per-file fixed point over asserting TEMPLATES.
    asserting = set(SEEDS)
    templates = list(routine_blocks(lines, TEMPLATE_RE))
    changed = True
    while changed:
        changed = False
        for name, _ln, body in templates:
            if name in asserting:
                continue
            if (idents(body) - shadowed(body)) & asserting:
                asserting.add(name)
                changed = True

    findings = []
    for name, ln, body in routine_blocks(lines, ROUTINE_RE):
        hit = sorted((idents(body) - shadowed(body)) & asserting)
        if hit:
            findings.append((name, ln, ",".join(hit)))
    return findings


CONTROL = '''
import std/unittest

template ck(condition: untyped) =
  check condition

template ckWrap(condition: untyped) =
  ck condition

proc directDefect(a, b: int) =
  check a == b

proc oneLevelWrapper(a, b: int) =
  ck a == b

proc twoLevelWrapper(a, b: int) =
  ckWrap a == b

proc proseOnly() =
  # check a == b
  echo "check a == b"

proc qualifiedCall(m: MockService) =
  m.expect("ping")

proc localVariableNamedLikeTemplate() =
  var ck = @["a"]
  while ck.len > 0: discard ck.pop()
'''

CONTROL_MUST_FIND = {"directDefect", "oneLevelWrapper", "twoLevelWrapper"}
CONTROL_MUST_NOT_FIND = {
    "proseOnly",
    "qualifiedCall",
    "localVariableNamedLikeTemplate",
}


def run_control(tmpdir: Path) -> tuple[bool, str]:
    f = tmpdir / "trap13_control.nim"
    f.write_text(CONTROL, encoding="utf-8")
    found = {name for name, _ln, _why in sweep_file(f)}
    missed = CONTROL_MUST_FIND - found
    spurious = CONTROL_MUST_NOT_FIND & found
    ok = not missed and not spurious
    return ok, f"found={sorted(found)} missed={sorted(missed)} spurious={sorted(spurious)}"


def main(argv: list[str]) -> int:
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        ok, detail = run_control(Path(td))
    print(f"planted control: {'PASS' if ok else 'FAIL'} — {detail}")
    if not ok:
        print("REFUSING TO REPORT: the sweep has stopped reading Nim.")
        return 2

    roots = [Path(a) for a in argv[1:]] or [Path("tests"), Path("src")]
    files = []
    for r in roots:
        if r.is_file():
            files.append(r)
        else:
            files.extend(sorted(r.rglob("*.nim")))
    if not files:
        print("REFUSING TO REPORT: the subject list is empty (trap 6a floor).")
        return 2
    print(f"subjects scanned: {len(files)}")

    total = 0
    for f in files:
        for name, ln, why in sweep_file(f):
            print(f"TRAP13  {f}:{ln}  proc {name}  reaches: {why}")
            total += 1
    print(f"findings: {total}")
    return 1 if total else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
