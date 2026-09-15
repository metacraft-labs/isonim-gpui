#!/usr/bin/env python3
"""PLAT-19 mutation harness for isonim-gpui.

Every claim PLAT-19 makes is graded here by planting the defect back and
requiring the named case to go red FOR THE ARM'S OWN REASON. The rules it
implements come from `codetracer-specs/Testing/Verification-Harness-Traps.md`;
the ones that shaped the code are called out where they bite.

SEVEN VERDICTS (§1a, §16, §17). Two or three is not enough:

  KILLED                the killer case went red AND its failure text
                        carries the arm's `because`
  SURVIVED              the killer case stayed green — the evidence does
                        not reach the code the arm changed
  NEVER-RAN             the mutation did not apply, or the subject did not
                        compile. §1a: an rc-based harness folds this into
                        KILLED, because `nim c -r` exits non-zero for a
                        compile error and for a failed assertion alike
  MIS-ATTRIBUTED        the case went red for a DIFFERENT reason (§17) —
                        `the run told you nothing`, not `the code is wrong`
  NO-VERDICT-FOR-KILLER the killer appears in neither the passed nor the
                        failed list. §1a's mirror: a mutant that CRASHES
                        the binary prints no verdict line, and a line
                        parser folds that into SURVIVED
  CONTROL-FAILURE       the arm's NAMED CONTROL also went red, so the arm
                        is a blanket break rather than an aimed one and
                        its kill is not evidence about the claim
  HARNESS-FAILURE       the needle did not occur exactly once, a restore
                        did not reproduce the recorded digest, or a
                        grader could not be run

ALL SEVEN ARE REACHABLE in this arm set. NEVER-RAN and
NO-VERDICT-FOR-KILLER need a compiled subject and are reachable through
the Nim and Rust arms (W1-W5, E1, B1); the shell/python gate arms (G1-G5,
T1, L1, L2) cannot reach NEVER-RAN, because there is nothing to compile —
which is why the arm set contains both kinds rather than only the cheap
kind.

`because` IS DERIVED, NEVER TYPED (§17a, §17b). Run `--derive-because`:
each arm is applied, its grader run, and the failure text printed. Paste
THAT. A string typed from the source is a second copy of the code held in
a file the compiler does not read, and for a `unittest` assertion inside a
template it is a string that can never occur, because `unittest` prints
the AST AFTER substitution. The harness additionally REFUSES a run in
which two arms derived the same `because`: two arms sharing a quotation
can be attributed to each other's cases.

ORDERING (§16): needle-scan, THEN re-record digests, THEN re-run every
arm. `--record-control-hashes` is GATED on the needle scan, because
re-recording from a subject under suspicion is §7 arriving inside the
verification step.

LOCKING (§14d): a `flock` is taken before ANY digest is read or written
and held for the whole run, and it is taken SHARED-EXCLUSIVE (there is
only one writer here, so exclusive throughout). The lock file is in
`.gitignore`. Do not `rm -f` it — `flock` is on the inode, so unlinking
lets the next starter lock a fresh inode and serialise against nothing.
Poll it with `flock -n`, never with `pgrep -f`, which matches its own
watcher shells.

TOUCHED (§16b/§16c): this list names every file an arm MUTATES **and**
every suite or gate an arm is GRADED AGAINST. `TOUCHED` is naturally a
restore set, so nine of sixteen harnesses in the sibling repository name
their subjects and not their graders — and a change that touches only a
grader then produces no overlap signal while invalidating every arm
graded against it. This one names both, and says so.
"""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import os
import re
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
LOCK_PATH = REPO / ".plat19-mutations.lock"
DIGESTS = REPO / "tools" / "plat19-mutation-control.sha256"

# --- subjects (files an arm mutates) -----------------------------------
WINDOW_NIM = "src/isonim_gpui/window.nim"
RENDERER_NIM = "src/isonim_gpui/renderer.nim"
BINDINGS_NIM = "src/isonim_gpui/bindings.nim"
WINDOW_RS = "rust/gpui-nim-shim/src/window.rs"
LANE = "ci/run-suite.sh"
CHECK_BINDINGS = "tools/check_bindings.sh"
CHECK_PIN = "tools/check_gpui_pin.sh"
TRAP13 = "tools/trap13-assertion-helper-sweep.py"
CARGO_LOCK = "rust/Cargo.lock"
PIN_DOC = "docs/gpui-pin.md"
SHIM_TOML = "rust/gpui-nim-shim/Cargo.toml"
HEADLESS_RS = "rust/gpui-nim-shim/src/gpui_headless.rs"

# --- graders (suites and gates an arm's verdict depends on) ------------
SUITE_WINDOW_REGISTRY = "tests/test_window_callback_registry.nim"
SUITE_RENDERER = "tests/test_renderer.nim"
SUITE_BINDINGS = "tests/test_bindings.nim"
SUITE_STRUCTURAL = "tests/test_structural_comparison.nim"
SUITE_ASYNC_RENDER = "rust/gpui-nim-shim/tests/test_async_render.rs"

# §16c: SUBJECTS **and** GRADERS. The graders are here even though no arm
# restores them, because a commit that touches only a grader invalidates
# every arm graded against it and a `TOUCHED ∩ diff` sweep would report
# no overlap at all.
# The claim this list makes, stated so it can be checked: EVERY source
# file of this repository that an arm's verdict depends on. Not only the
# eleven an arm restores.
#
# The lane arms (L1, L2) are graded by `ci/run-suite.sh`, whose TOTAL
# CASES depends on every suite it runs and on `nim.cfg`, which decides
# whether three of them compile at all. The Rust arms (W4, W5) are graded
# by `cargo test`, which compiles every module of the crate. Both sets are
# here.
#
# DELIBERATELY OUTSIDE, each for a stated reason rather than by omission
# (§7a: the undefended site is the defect, so every exclusion gets a
# sentence):
#   * `flake.nix` — the dev SHELL, not a source file. It does reach a
#     grader (the trap-13 gate needs python3 from it), but putting it in
#     would force a re-record on every unrelated toolchain change, and a
#     digest set that is re-recorded routinely stops being a control.
#   * `.github/workflows/ci.yml` — not executed by any arm; the lane is.
#   * `tools/run-plat19-mutations.py` and its digest file — the
#     instrument itself. A harness cannot hold its own control bytes.
#
# NAMED RESIDUAL, so it is counted rather than rediscovered: the suites
# import `../isonim/src` and `../nim-everywhere/src`, which are other
# repositories. A change there can redden an arm's grader with nothing in
# this list moving. That is the same transitive blind spot PLAT-14 records
# for its own suites; closing it needs a cross-repo enumeration, not a
# longer list here.
TOUCHED = [
    # --- mutation subjects (the restore set) ---
    WINDOW_NIM,
    RENDERER_NIM,
    BINDINGS_NIM,
    WINDOW_RS,
    LANE,
    CHECK_BINDINGS,
    CHECK_PIN,
    TRAP13,
    CARGO_LOCK,
    PIN_DOC,
    SHIM_TOML,
    # --- graders: the Nim suites an arm's verdict is read from ---
    SUITE_WINDOW_REGISTRY,
    SUITE_RENDERER,
    SUITE_BINDINGS,
    SUITE_STRUCTURAL,
    # --- graders: the rest of what the LANE runs, because L1/L2 grade
    #     against its total and its per-subject rcs ---
    "tests/test_basic.nim",
    "tests/test_cross_renderer.nim",
    "tests/test_gui.nim",
    "tests/test_render_integration.nim",
    "tests/test_performance.nim",
    "nim.cfg",
    # --- graders: the rest of the Rust crate, because W4/W5 grade
    #     against `cargo test`, which compiles all of it ---
    "rust/gpui-nim-shim/src/lib.rs",
    "rust/gpui-nim-shim/src/tree.rs",
    "rust/gpui-nim-shim/src/render_sync.rs",
    "rust/gpui-nim-shim/src/gpui_app.rs",
    "rust/gpui-nim-shim/src/gpui_headless.rs",  # also H1's SUBJECT
    # --- grader: the `gpui-headless` feature suite H1 grades against.
    #     It is `#![cfg(feature = "gpui-headless")]`, so the default
    #     `cargo test` the Rust arms use reports `running 0 tests` for
    #     it and W4/W5 are unaffected by it — but H1 reads its verdict
    #     lines, so §16c puts it here.
    SUITE_ASYNC_RENDER,
]

NIM_ENV = {
    **os.environ,
    "LD_LIBRARY_PATH": f"{REPO}/rust/target/debug:" + os.environ.get("LD_LIBRARY_PATH", ""),
}


@dataclass
class Arm:
    id: str
    claim: str
    file: str
    find: str
    replace: str
    grader: str  # key into GRADERS
    kills: str  # the case (or output needle) that must go red
    control: str  # NAMED control that must stay green
    kills_mode: str = "case"  # "case" | "absence" | "compile_error"
    because: str = ""  # DERIVED from a transcript, never typed
    rebuild_rust: bool = False
    control_find: str = ""
    verdict: str = ""
    detail: str = ""


def run(cmd, **kw):
    return subprocess.run(
        cmd, cwd=REPO, capture_output=True, text=True, env=NIM_ENV, **kw
    )


# --- graders ------------------------------------------------------------
#
# Each returns (passed_cases, failed_cases, transcript). For the gates,
# "cases" are output needles rather than unittest case names; a gate's
# `kills` is a substring its stderr must carry and its `control` a
# substring its stdout must still carry.


def grade_nim(path: str):
    cache = f"/tmp/plat19mut/{Path(path).stem}"
    p = run(
        ["nim", "c", "-r", "--hints:off", "--warnings:off", f"--nimcache:{cache}", path]
    )
    out = p.stdout + p.stderr
    # §1a: `nim c -r` exits non-zero for a compile error, for a failed
    # assertion and for a process that aborted, all the same number — so
    # the rc cannot tell them apart and the TRANSCRIPT must. A run that
    # produced verdict lines, an unhandled exception, or a failed child
    # process has RUN; only an `Error:` with none of those is a mutant
    # that never got as far as executing.
    ran = (
        "[OK]" in out
        or "[FAILED]" in out
        or "unhandled exception" in out
        or "execution of an external program failed" in out
    )
    if "Error: " in out and not ran:
        return None, None, out  # did not compile -> NEVER-RAN
    passed = set(re.findall(r"^\s*\[OK\] (.+)$", out, re.M))
    failed = set(re.findall(r"^\s*\[FAILED\] (.+)$", out, re.M))
    # assert-style scripts have no case lines; use checkpoints as cases
    if not passed and not failed:
        passed = set(re.findall(r"^([^ :]+: .*(?:passed|verified).*)$", out, re.M))
    return passed, failed, out


def grade_rust():
    p = run(["cargo", "test", "--manifest-path", "rust/Cargo.toml"])
    out = p.stdout + p.stderr
    if "error[" in out or "error: could not compile" in out:
        return None, None, out
    passed = set(re.findall(r"^test (\S+) \.\.\. ok$", out, re.M))
    failed = set(re.findall(r"^test (\S+) \.\.\. FAILED$", out, re.M))
    return passed, failed, out


def grade_rust_headless():
    """`cargo test --features gpui-headless --test test_async_render`.

    A SEPARATE grader from `grade_rust` because the async-render suite is
    `#![cfg(feature = "gpui-headless")]` — under the default `cargo test`
    it reports `running 0 tests`, and a universal quantification over an
    empty case set is a pass (§4). An arm graded against the default
    build could therefore never be killed by anything in that file.
    """
    p = run(
        [
            "cargo",
            "test",
            "--manifest-path",
            "rust/Cargo.toml",
            "--features",
            "gpui-headless",
            "--test",
            "test_async_render",
        ]
    )
    out = p.stdout + p.stderr
    if "error[" in out or "error: could not compile" in out:
        return None, None, out
    passed = set(re.findall(r"^test (\S+) \.\.\. ok$", out, re.M))
    failed = set(re.findall(r"^test (\S+) \.\.\. FAILED$", out, re.M))
    # §4's floor, because this grader can silently address zero cases if
    # the feature gate or the `--test` target name ever moves.
    if not passed and not failed:
        return None, None, out
    return passed, failed, out


def grade_shell(script: str, args=None):
    p = run(["bash", script] + (args or []))
    return p.returncode, p.stdout + p.stderr


def grade_python(script: str, args=None):
    p = run(["python3", script] + (args or []))
    return p.returncode, p.stdout + p.stderr


GRADERS = {
    "window_registry": lambda: grade_nim(SUITE_WINDOW_REGISTRY),
    "renderer": lambda: grade_nim(SUITE_RENDERER),
    "bindings": lambda: grade_nim(SUITE_BINDINGS),
    "rust": grade_rust,
    "rust_headless": grade_rust_headless,
    "lane": lambda: grade_shell(LANE),
    "check_bindings": lambda: grade_shell(CHECK_BINDINGS),
    "check_pin": lambda: grade_shell(CHECK_PIN),
    "trap13": lambda: grade_python(TRAP13, ["tests", "src"]),
}
GATE_GRADERS = {"lane", "check_bindings", "check_pin", "trap13"}


# --- the arms -----------------------------------------------------------

ARMS: list[Arm] = [
    Arm(
        id="W1",
        claim="window lifecycle callbacks are keyed by WINDOW ID, so N windows "
        "do not share one slot",
        file=WINDOW_NIM,
        find="  resizeCallbacks[win.id] = callback",
        replace="  resizeCallbacks[0'u32] = callback",
        grader="window_registry",
        kills="20 windows x 3 callback kinds all register and all route to their own window",
        # NOT `resetWindows clears the registry`: that case asserts
        # `windowCallbackCount() == 1`, and keying every resize under a
        # constant legitimately makes it 2 (keys {0, win.id}) — so the
        # first control this arm was given went red WITH the killer and
        # the harness scored CONTROL-FAILURE, correctly. A control has to
        # be a case the arm cannot reach; W1 touches only the resize
        # registration, so the close path is one.
        control="onClose is routed per window: denying on one does not deny on the other",
        because="Check failed: resized[][i] == 1\n    resized[][i] was 0",
    ),
    Arm(
        id="W2",
        claim="the dispatcher delivers an event to ONE window, not to every "
        "registered closure",
        file=WINDOW_NIM,
        find="  let cb = resizeCallbacks.getOrDefault(windowId)\n  if cb != nil: cb(w.float, h.float)",
        replace="  for cb in resizeCallbacks.values:\n    if cb != nil: cb(w.float, h.float)",
        grader="window_registry",
        kills="NEGATIVE CONTROL: notifying ONE window moves exactly that window's counters",
        control="onClose is routed per window: denying on one does not deny on the other",
        because="Check failed: resized[][i] ==\n  if i == Chosen:\n    1\n  else:\n    0\n    resized[][i] was 1",
    ),
    Arm(
        id="W3",
        claim="destroying a window RELEASES its registry entries — a registry "
        "trades a pool's hard bound for a leak unless it does",
        file=WINDOW_NIM,
        find="  releaseWindowCallbacks(win.id)\n  gpui_destroy_window(win.id)",
        replace="  gpui_destroy_window(win.id)",
        grader="window_registry",
        kills="destroying a window releases its registry entries",
        control="resetWindows clears the registry",
        because="Check failed: windowCallbackCount() == 0",
    ),
    Arm(
        id="W4",
        claim="the shim consults the dispatcher only for windows that OPTED IN",
        file=WINDOW_RS,
        find="    if dispatched {\n        if let Some(d) = *take_lock(&RESIZE_DISPATCHER) {\n            d(id, width, height);\n            return;\n        }\n    }",
        replace="    if true {\n        if let Some(d) = *take_lock(&RESIZE_DISPATCHER) {\n            d(id, width, height);\n            return;\n        }\n    }",
        grader="rust",
        kills="window::tests::test_dispatcher_is_not_consulted_for_windows_that_did_not_opt_in",
        control="window::tests::test_dispatcher_routes_by_window_id",
        # §17a/§17b, measured here on 2026-09-15. The first derived
        # `because` for this arm was the assert_eq body —
        #     assertion `left == right` failed
        #       left: [(11, 11), (12, 22)]
        #      right: [(11, 11)]
        # — and it scored MIS-ATTRIBUTED on the very next run over a
        # mutation that killed the case exactly as intended. Window ids
        # come from a process-global `NEXT_WINDOW_ID` counter, so `11`
        # and `12` are a function of how many windows the SUITE created
        # before this test, and widening TOUCHED (which changed nothing
        # about the window code) was enough to move them. That is §17a's
        # gensym hazard wearing different clothes: a `because` naming a
        # value that is not stable is not merely wrong, it is wrong
        # INTERMITTENTLY. The stable quotation is the panicking test's
        # own name, which is also the thing §17 asks the verdict to
        # attribute — did THIS case die, or something upstream of it.
        because="thread 'window::tests::test_dispatcher_is_not_consulted_for_windows_that_did_not_opt_in'",
    ),
    Arm(
        id="W5",
        claim="an id no window owns reaches no registered closure",
        file=WINDOW_RS,
        find="    let Some((dispatched, legacy)) = with_window(id, |w| (w.dispatch_focus, w.on_focus)) else {\n        return;\n    };",
        replace="    let (dispatched, legacy) =\n        with_window(id, |w| (w.dispatch_focus, w.on_focus)).unwrap_or((true, None));",
        grader="rust",
        kills="window::tests::test_unknown_window_id_reaches_no_dispatcher",
        control="window::tests::test_dispatcher_routes_by_window_id",
        because="assertion failed: log().is_empty()",
    ),
    Arm(
        id="H1",
        claim="a token abandoned while its render is in flight is TOMBSTONED, "
        "so the worker's pending publish is dropped instead of orphaning a "
        "slot nobody will poll again",
        file=HEADLESS_RS,
        find="fn abandon_slot(state: &WorkerState, slots: &mut HashMap<u32, RenderSlot>, token: u32) {\n    state.abandons.fetch_add(1, Ordering::AcqRel);\n    slots.insert(token, RenderSlot::Abandoned);\n}",
        replace="fn abandon_slot(state: &WorkerState, slots: &mut HashMap<u32, RenderSlot>, token: u32) {\n    state.abandons.fetch_add(1, Ordering::AcqRel);\n    let _ = (slots, token);\n}",
        grader="rust_headless",
        kills="abandoned_token_leaves_no_orphan_slot",
        # NOT `stale_token_after_bump_returns_stale_sentinel`: this
        # mutation reddens that case too (deterministically, now that
        # the tombstone is what makes its re-poll unambiguous), so it
        # would score CONTROL-FAILURE — an arm is only evidence if its
        # control survives. `async_try_take_unknown_token` runs through
        # the same entry point and does not touch the stale branch.
        control="async_try_take_unknown_token",
        # The arm removes the tombstone INSERT and keeps the counter, so
        # the killer case can still prove it reached the in-flight branch
        # — otherwise the mutant would make the case vacuous rather than
        # red, and a vacuous case is §4's pass, not a survival.
        #
        # This is also the arm that shows why the counter is in the
        # product rather than in the test: without it the case waits on
        # the SLOT MAP, which the mutation empties, so the wait returns
        # before the publish and the case passes over the defect.
        # Measured — that is exactly what the first red-first attempt
        # did, on 2026-09-15.
        because="thread 'abandoned_token_leaves_no_orphan_slot'",
    ),
    Arm(
        id="E1",
        claim="the ELEMENT callback registry has no sixteen-slot bound — this "
        "arm puts the bound back",
        file=RENDERER_NIM,
        find="  let id = nextCallbackId\n  inc nextCallbackId",
        replace="  let id = ((nextCallbackId - 1'i32) mod 16'i32) + 1'i32\n  inc nextCallbackId",
        grader="renderer",
        kills="test_renderer: dynamic callback scalability test passed (150 callbacks)",
        kills_mode="absence",
        control="test_renderer: callback registry unit test passed",
        because="`counters.data[i] == 1` callback 0 should have fired once, got: 0",
    ),
    Arm(
        id="B1",
        claim="test_bindings refuses a binding that is declared and never "
        "exercised — the completeness claim its header used to make for free",
        file=BINDINGS_NIM,
        find="proc gpui_reset_windows*()\n  {.importc: \"gpui_reset_windows\".}",
        replace="proc gpui_plat19_unverified_probe*()\n  {.importc: \"gpui_reset_windows\".}\n\nproc gpui_reset_windows*()\n  {.importc: \"gpui_reset_windows\".}",
        grader="bindings",
        kills="test_bindings: every binding declared in bindings.nim is verified at compile time",
        kills_mode="absence",
        control="__no_floor_failure__",
        because="test_bindings: these bindings are declared but never exercised here: gpui_plat19_unverified_probe",
    ),
    Arm(
        id="L1",
        claim="the lane's case-count assertion catches a suite that stopped "
        "running — which does not fail, it ceases to exist",
        file=LANE,
        find="\ttests/test_structural_comparison.nim\n",
        replace="",
        grader="lane",
        kills="CASE COUNT DRIFT",
        control="LANE RESULT: FAILED",
        because="CASE COUNT DRIFT: the lane ran 231 cases, not 237.",
    ),
    Arm(
        id="L2",
        claim="the lane takes its VERDICT from the exit code, so a red subject "
        "is named even while the case COUNT is unchanged",
        file=SUITE_STRUCTURAL,
        find="check plan[\"kind\"].getStr() == \"Div\"",
        replace="check plan[\"kind\"].getStr() == \"DivPLAT19\"",
        grader="lane",
        kills="nim: tests/test_structural_comparison.nim",
        control="TOTAL CASES: 237   (expected 237)",
        because="- nim: tests/test_structural_comparison.nim (rc=1)",
    ),
    Arm(
        id="G1",
        claim="check_bindings REFUSES rather than reporting a clean sweep when "
        "its scan reads nothing (traps 4 / 6a)",
        file=CHECK_BINDINGS,
        find="RUST_FUNCS=$(grep -rhoP 'pub (?:unsafe )?extern \"C\" fn \\K\\w+' \"$RUST_SRC_DIR\"/*.rs | sort -u || true)",
        replace="RUST_FUNCS=$(grep -rhoP 'pub NOTHING_MATCHES_THIS extern \"C\" fn \\K\\w+' \"$RUST_SRC_DIR\"/*.rs | sort -u || true)",
        grader="check_bindings",
        kills="REFUSING TO REPORT",
        control="__rc_is_2__",
        because="REFUSING TO REPORT: found 0 Rust exports, floor is 40.",
    ),
    Arm(
        id="G2",
        claim="check_bindings scans the whole crate — narrowed to lib.rs it "
        "reports the six gpui_headless exports as EXTRA, which is what it did "
        "at HEAD",
        file=CHECK_BINDINGS,
        find="\"$RUST_SRC_DIR\"/*.rs | sort -u || true)",
        replace="\"$RUST_SRC_DIR\"/lib.rs | sort -u || true)",
        grader="check_bindings",
        kills="EXTRA in Nim bindings",
        control="Rust extern \"C\" exports:",
        because="EXTRA in Nim bindings (present in Nim but not in Rust):",
    ),
    Arm(
        id="G3",
        claim="the pin gate catches a manifest whose copies of the revision "
        "disagree",
        file=SHIM_TOML,
        find='gpui_platform = { git = "https://github.com/zed-industries/zed", rev = "562a0e03b5c3c6e696154de92fd56302a41683c4", optional = true }',
        replace='gpui_platform = { git = "https://github.com/zed-industries/zed", rev = "0000000000000000000000000000000000000000", optional = true }',
        grader="check_pin",
        kills="Pinned revisions disagree",
        control="__rc_is_1__",
        because="Pinned revisions disagree across rust/**/Cargo.toml:",
    ),
    Arm(
        id="G4",
        claim="the pin gate reads the LOCK, so a manifest bumped without "
        "`cargo update` is red rather than quietly building the old tree",
        file=CARGO_LOCK,
        find='name = "gpui_platform"\nversion = "0.1.0"\nsource = "git+https://github.com/zed-industries/zed?rev=562a0e03b5c3c6e696154de92fd56302a41683c4#562a0e03b5c3c6e696154de92fd56302a41683c4"',
        replace='name = "gpui_platform"\nversion = "0.1.0"\nsource = "git+https://github.com/zed-industries/zed?rev=1111111111111111111111111111111111111111#1111111111111111111111111111111111111111"',
        grader="check_pin",
        kills="Cargo.lock resolved MORE THAN ONE zed revision",
        control="manifests: 3 pin(s)",
        because="Cargo.lock resolved MORE THAN ONE zed revision:",
    ),
    Arm(
        id="G5",
        claim="the pin gate reads the DOCUMENT, so the procedure a reader "
        "trusts cannot fall behind the build",
        file=PIN_DOC,
        find="**Pinned revision:** `zed-industries/zed@562a0e03b5c3c6e696154de92fd56302a41683c4`",
        replace="**Pinned revision:** `zed-industries/zed@2222222222222222222222222222222222222222`",
        grader="check_pin",
        kills="the documented revision is not",
        control="lock:      23 zed source line(s)",
        because="the documented revision is not 562a0e03b5c3c6e696154de92fd56302a41683c4 — the headline names 2222222222222222222222222222222222222222",
    ),
    Arm(
        id="T1",
        claim="the trap-13 sweep iterates to a FIXED POINT — a wrapper of a "
        "wrapper is the shape 13a says recurs, and the planted control "
        "refuses the run when the sweep stops finding it",
        file=TRAP13,
        find="            if (idents(body) - shadowed(body)) & asserting:",
        replace="            if (idents(body) - shadowed(body)) & SEEDS:",
        grader="trap13",
        kills="REFUSING TO REPORT",
        control="planted control: FAIL",
        because="REFUSING TO REPORT: the sweep has stopped reading Nim.",
    ),
]


# --- mechanics ----------------------------------------------------------


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def needle_scan() -> list[str]:
    """§16: every arm whose `find` does not occur EXACTLY ONCE in its file."""
    bad = []
    for a in ARMS:
        p = REPO / a.file
        if not p.exists():
            bad.append(f"{a.id}: subject does not exist: {a.file}")
            continue
        n = p.read_text().count(a.find)
        if n != 1:
            bad.append(f"{a.id}: `find` occurs {n} times in {a.file}, expected exactly 1")
        if a.control_find:
            m = p.read_text().count(a.control_find)
            if m != 1:
                bad.append(
                    f"{a.id}: `control_find` occurs {m} times in {a.file}, expected exactly 1"
                )
    return bad


def subjects() -> list[str]:
    seen, out = set(), []
    for f in TOUCHED:
        if f not in seen:
            seen.add(f)
            out.append(f)
    return out


def record_hashes():
    lines = [f"{digest(REPO / f)}  {f}" for f in subjects()]
    DIGESTS.write_text("\n".join(lines) + "\n")
    print(f"recorded {len(lines)} control digests to {DIGESTS.relative_to(REPO)}")


def check_hashes() -> list[str]:
    if not DIGESTS.exists():
        return ["no recorded control digests — run --record-control-hashes"]
    bad = []
    for line in DIGESTS.read_text().splitlines():
        if not line.strip():
            continue
        want, f = line.split("  ", 1)
        p = REPO / f
        if not p.exists():
            bad.append(f"{f}: missing")
        elif digest(p) != want:
            bad.append(f"{f}\n      recorded {want[:8]}…   on disk {digest(p)[:8]}…")
    return bad


def rebuild_rust() -> bool:
    p = run(["cargo", "build", "--manifest-path", "rust/Cargo.toml"])
    return p.returncode == 0


def apply(a: Arm) -> str:
    p = REPO / a.file
    orig = p.read_text()
    assert orig.count(a.find) == 1
    p.write_text(orig.replace(a.find, a.replace, 1))
    return orig


def restore(a: Arm, orig: str) -> bool:
    p = REPO / a.file
    p.write_text(orig)
    return digest(p) == hashlib.sha256(orig.encode()).hexdigest()


def run_arm(a: Arm, derive: bool) -> None:
    orig = apply(a)
    try:
        if a.rebuild_rust or a.file.endswith(".rs"):
            if not rebuild_rust():
                a.verdict = "NEVER-RAN"
                a.detail = "the mutant did not compile"
                return
        g = GRADERS[a.grader]
        if a.grader in GATE_GRADERS:
            rc, out = g()
            if derive:
                a.detail = f"rc={rc}\n" + "\n".join(
                    ln for ln in out.splitlines() if a.kills in ln or "REFUS" in ln
                )
                return
            if a.kills not in out:
                a.verdict = "SURVIVED"
                a.detail = f"gate rc={rc}; `{a.kills}` absent from the transcript"
                return
            if a.because and a.because not in out:
                a.verdict = "MIS-ATTRIBUTED"
                a.detail = f"gate went red but not with `{a.because}`"
                return
            # named control
            ctl_ok = True
            if a.control == "__rc_is_2__":
                ctl_ok = rc == 2
            elif a.control == "__rc_is_1__":
                ctl_ok = rc == 1
            else:
                ctl_ok = a.control in out
            if not ctl_ok:
                a.verdict = "CONTROL-FAILURE"
                a.detail = f"named control `{a.control}` did not hold (rc={rc})"
                return
            a.verdict = "KILLED"
            a.detail = f"rc={rc}; control `{a.control}` held"
            return

        passed, failed, out = g()
        if derive and a.grader == "rust" and passed is not None:
            m = re.search(
                r"^---- " + re.escape(a.kills) + r" stdout ----$(.*?)(?=^----|\Z)",
                out,
                re.M | re.S,
            )
            blk = (m.group(1).strip() if m else out[-400:])
            a.detail = "\n".join(blk.splitlines()[:5]).rstrip()
            return
        if passed is None:
            # A compile failure. For B1 that IS the kill.
            if a.kills_mode == "compile_error":
                if derive:
                    a.detail = "\n".join(
                        ln for ln in out.splitlines() if "never exercised here" in ln
                    ) or out[-400:]
                    return
                if a.because and a.because not in out:
                    a.verdict = "MIS-ATTRIBUTED"
                    a.detail = f"compile error, but not `{a.because}`"
                    return
                if a.control == "__no_floor_failure__" and "extracted only" in out:
                    a.verdict = "CONTROL-FAILURE"
                    a.detail = "the FLOOR assertion fired, not the completeness one"
                    return
                a.verdict = "KILLED"
                a.detail = "compile error carrying the arm's because"
                return
            a.verdict = "NEVER-RAN"
            a.detail = "the mutant did not compile"
            return
        if a.kills_mode == "compile_error":
            a.verdict = "SURVIVED"
            a.detail = "expected a compile error; the subject compiled"
            return
        if derive:
            # §17a/§17b: the `because` must be the failure text of THIS
            # arm's killer case, not of the whole transcript — two arms
            # over one suite otherwise derive the same string and can be
            # attributed to each other's cases. `unittest` prints a
            # case's failures immediately ABOVE its verdict line, so the
            # scope is "since the previous verdict line".
            block: list[str] = []
            for ln in out.splitlines():
                if re.match(r"^\s*\[(OK|FAILED)\] ", ln):
                    if ln.strip().startswith("[FAILED] ") and ln.strip()[9:] == a.kills:
                        break
                    block = []
                    continue
                block.append(ln)
            idxs = [i for i, ln in enumerate(block) if "Check failed:" in ln]
            if idxs:
                a.detail = "\n".join(block[idxs[0] : idxs[0] + 8]).rstrip()
            else:
                abort = [ln for ln in out.splitlines() if "unhandled exception" in ln]
                a.detail = "\n".join(abort[:2]) or "\n".join(block[-8:]) or out[-400:]
            return
        if a.kills_mode == "absence":
            # An assert-style script prints a checkpoint per block and
            # aborts on the first failure, so the SIGNAL is the
            # checkpoint's absence. Absence alone is not enough — it is
            # also what a suite that never ran looks like (§4) — so the
            # transcript must additionally carry the arm's own because.
            if a.kills in passed:
                a.verdict = "SURVIVED"
                a.detail = f"`{a.kills}` was still printed"
                return
            if a.because and a.because not in out:
                a.verdict = "MIS-ATTRIBUTED"
                a.detail = f"the checkpoint is gone, but not because of `{a.because}`"
                return
            if a.control == "__no_floor_failure__":
                # The NAMED control for B1: the failure must be the
                # COMPLETENESS assertion, not the non-vacuity floor above
                # it. Without this the arm would be satisfied by an
                # extractor that had stopped reading bindings.nim, which
                # is the very state the floor exists to refuse.
                if "extracted only" in out:
                    a.verdict = "CONTROL-FAILURE"
                    a.detail = "the FLOOR assertion fired, not the completeness one"
                    return
            elif a.control not in passed:
                a.verdict = "CONTROL-FAILURE"
                a.detail = f"named control `{a.control}` did not stay green"
                return
            a.verdict = "KILLED"
            a.detail = f"checkpoint absent with the arm's because; control `{a.control}` held"
            return
        if a.kills not in passed and a.kills not in failed:
            a.verdict = "NO-VERDICT-FOR-KILLER"
            a.detail = (
                f"`{a.kills}` is in neither the passed nor the failed list — "
                "the run told you nothing (the mutant may have crashed the binary)"
            )
            return
        if a.kills in passed:
            a.verdict = "SURVIVED"
            a.detail = f"`{a.kills}` stayed green"
            return
        if a.because and a.because not in out:
            a.verdict = "MIS-ATTRIBUTED"
            a.detail = f"`{a.kills}` went red, but not with `{a.because}`"
            return
        if a.control not in passed:
            a.verdict = "CONTROL-FAILURE"
            a.detail = f"named control `{a.control}` did not stay green"
            return
        a.verdict = "KILLED"
        a.detail = f"control `{a.control}` stayed green"
    finally:
        ok = restore(a, orig)
        if not ok:
            a.verdict = "HARNESS-FAILURE"
            a.detail = "restore did not reproduce the original bytes"
        if a.file.endswith(".rs"):
            rebuild_rust()


def enumerate_touched():
    """§16b: intersect TOUCHED with the DIFF BEING COMMITTED, not with the
    session's mental delta — `-uall`, so untracked files count."""
    p = subprocess.run(
        ["git", "status", "--porcelain", "-uall"],
        cwd=REPO,
        capture_output=True,
        text=True,
    )
    diff = {ln[3:].strip() for ln in p.stdout.splitlines() if ln.strip()}
    overlap = sorted(set(subjects()) & diff)
    print(f"git status -uall reports {len(diff)} changed path(s) in isonim-gpui")
    print(f"TOUCHED names {len(subjects())} path(s); {len(overlap)} overlap:")
    for f in overlap:
        print(f"  {f}")
    missing = sorted(set(subjects()) - diff)
    if missing:
        print("TOUCHED paths NOT in this diff (arms over unchanged files):")
        for f in missing:
            print(f"  {f}")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--needle-scan", action="store_true")
    ap.add_argument("--record-control-hashes", action="store_true")
    ap.add_argument("--derive-because", action="store_true")
    ap.add_argument("--enumerate-touched", action="store_true")
    ap.add_argument("--only", default="")
    args = ap.parse_args()

    if args.enumerate_touched:
        enumerate_touched()
        return 0

    # §14d: take the lock BEFORE reading or writing any digest, and hold
    # it for the whole run. flock is on the inode; never `rm -f` it.
    lock = open(LOCK_PATH, "w")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        print("another PLAT-19 mutation run holds the lock; refusing to start.")
        return 3

    bad = needle_scan()
    if args.needle_scan:
        if bad:
            print("NEEDLE SCAN FAILED:")
            for b in bad:
                print("  " + b)
            return 1
        print(f"needle scan: all {len(ARMS)} arms resolve to exactly one site.")
        return 0

    if bad:
        print("NEEDLE SCAN FAILED — refusing to run or record:")
        for b in bad:
            print("  " + b)
        return 1

    if args.record_control_hashes:
        record_hashes()
        return 0

    drift = check_hashes()
    if drift and not args.derive_because:
        print("TREE IS NOT AT THE CONTROL BYTES — nothing was mutated.")
        for d in drift:
            print("  " + d)
        return 1

    # §17b: two arms deriving the same `because` can be attributed to each
    # other's cases. Refuse rather than report.
    if not args.derive_because:
        seen: dict[str, str] = {}
        for a in ARMS:
            if not a.because:
                print(f"ARM {a.id} HAS NO DERIVED `because` — run --derive-because.")
                return 1
            if a.because in seen:
                print(
                    f"REFUSING: arms {seen[a.because]} and {a.id} derived the SAME "
                    f"`because`:\n  {a.because}"
                )
                return 1
            seen[a.because] = a.id

    selected = [a for a in ARMS if not args.only or a.id in args.only.split(",")]
    for a in selected:
        print(f"--- {a.id}  {a.claim}")
        run_arm(a, args.derive_because)
        if args.derive_because:
            print(f"    DERIVED because candidate:\n{a.detail}\n")
        else:
            print(f"    {a.verdict}: {a.detail}")

    if args.derive_because:
        return 0

    print("\n=== PLAT-19 mutation tally ===")
    tally: dict[str, int] = {}
    for a in selected:
        tally[a.verdict] = tally.get(a.verdict, 0) + 1
        print(f"{a.id:4s} {a.verdict:22s} {a.claim[:70]}")
    print("-----------------------------")
    for k in sorted(tally):
        print(f"{k:22s} {tally[k]}")

    post = check_hashes()
    if post:
        print("\nPOST-RUN DIGEST CHECK FAILED — the tree did not come back:")
        for d in post:
            print("  " + d)
        return 1
    print("\npost-run digest check: every subject is byte-identical to its control.")
    return 0 if tally.get("KILLED", 0) == len(selected) else 1


if __name__ == "__main__":
    sys.exit(main())
