# `gpui-headless`'s async render worker builds the platform off the main thread, so every async render on macOS returns `-6`

| | |
|---|---|
| Status | open |
| Recorded | 2026-10-02 |
| Observed in | isonim-gpui @ `4b3ed3e` (`agents`), aarch64-darwin (macOS 15), shim built `--features gpui-headless` |
| Area | `rust/gpui-nim-shim/src/gpui_headless.rs` `spawn_worker` / `worker_main` / `render_via_cached`; the lane step `rust: cargo test --features gpui-headless` in `ci/run-suite.sh --with-gpui` |
| Found by | PLAT-37 review on the fleet's macOS host, running `ci/run-suite.sh --with-gpui` |

**Archive searched before filing.** This repository's `issues/` has held
exactly one file ever (`git log --all -- 'issues/*'` → `159df3e` only, and
`git log --diff-filter=D -- issues/` is empty), and
`git log -i -S'async_render' -- issues/` returns nothing. The consumer-side
archive was searched too: `codetracer-specs` has
`2026-09-30-isonim-gpui-gpui-rendering-test-does-not-compile.md` (a different
target, and already fixed by `4b3ed3e`) and nothing on `async_render`,
`gpui-headless` or `render_submit_async` — `git log -i -S'test_async_render'
-- issues/` there is empty. **This defect is filed nowhere.**

## Observed

`cargo test --features gpui-headless` is red on macOS, 5 of 7:

```
running 7 tests
test abandoned_token_leaves_no_orphan_slot ... ok
test async_render_produces_non_empty_buffer ... FAILED
test async_submit_is_non_blocking ... FAILED
test async_try_take_unknown_token ... ok
test bump_generation_is_monotonic ... ok
test cancel_while_pending_leaves_no_orphan_slot ... ok
test stale_token_after_bump_returns_stale_sentinel ... ok
test result: FAILED. 5 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out
```

Both failures are the same number:

```
assertion `left == right` failed: async render must succeed on macOS, got -6
  left: -6
 right: 0
```

`-6` is `-(ErrorCode::Panic)` — `gpui_headless.rs`'s own code for *"a Rust
panic propagated up the FFI boundary"*. With `--nocapture` the panic it is
reporting is visible, and it is not in this repository's code:

```
thread 'isonim-gpui-render' panicked at
  gpui-pre-macos-0.3.5/src/platform.rs:210:46:
Mac platform not created on main thread
  3: core::option::expect_failed
  4: <core::option::Option<objc2::main_thread_marker::MainThreadMarker>>::expect
  5: <gpui_macos::platform::MacPlatform>::new
  6: gpui_platform::current_platform
  7: gpui_nim_shim::gpui_headless::render_via_cached
  8: gpui_nim_shim::gpui_headless::worker_main::{closure#1}
```

So the async path's dedicated worker thread (`isonim-gpui-render`) calls
`current_platform`, `MacPlatform::new` asks for a `MainThreadMarker`, and
there is none on a spawned thread. The `catch_unwind` in `worker_main` turns
that into `ErrorCode::Panic`, which is why the symptom is a return code and
not a crash.

**The SYNCHRONOUS headless path on the same host is fine**, which is what
narrows this to the worker thread rather than to the platform:
`codetracer/ci/test/plat37_headless_probe.nim` against the same
`--features gpui-headless` build answers `rc 0 / "ok"`, 256,000 bytes of
256,000 expected, 256,000 non-zero, 21 distinct byte values. That call runs
on the process's main thread.

## Expected

`test_async_render.rs`'s own assertion, which is `#[cfg]`-split by platform
and therefore states the contract per platform: `assert_eq!(rc, -2)` off
macOS (`RendererUnavailable`, the documented Linux answer) and
`assert_eq!(rc, 0, "async render must succeed on macOS")` on it. macOS is the
supported headless target — `gpui_headless.rs`'s `ErrorCode::RendererUnavailable`
doc comment says so in as many words: *"macOS is the supported headless
target; Linux falls back to RS-M14b"*.

`ci/run-suite.sh`'s contract is the other half: the lane's verdict is its exit
code, and `--with-gpui` is red on macOS because of this step alone. On the
`agents` tip the other previously-failing step (`--features gpui-backend`,
`gpui_rendering.rs`) was repaired by `4b3ed3e`, so this is now the only thing
between that lane and green on this platform.

## Evidence

Measured 2026-10-02 on aarch64-darwin / macOS 15 at `4b3ed3e` with a clean
tree (the PLAT-37 macOS working tree was stashed and the same two cases failed
identically at `159df3e` without it, so this is not a regression from that
work):

```sh
nix develop --command bash -c 'ci/run-suite.sh --with-gpui'
#   rust: --features gpui-backend     rc=0   cases=176
#   rust: --features gpui-headless    rc=101 cases=154
#   TOTAL CASES: 290   (expected 290)
#   LANE RESULT: FAILED — 1 step(s):
#     - rust: cargo test --features gpui-headless (rc=101)

nix develop --command bash -c 'cd rust && RUST_BACKTRACE=1 cargo test \
  --features gpui-headless --test test_async_render \
  async_render_produces_non_empty_buffer -- --nocapture --test-threads=1'
#   the backtrace quoted above
```

The reading of `-6` is from `gpui_headless.rs`'s `ErrorCode` enum (`Panic = 6`)
and the negation convention at its call sites; the backtrace is measured, not
inferred.

## Suggested direction

Two shapes, and the choice is a design question this issue does not settle.

1. **Keep the worker thread and stop it from building a platform.** The
   worker exists so `gpui_render_submit_async` returns in microseconds
   (`async_submit_is_non_blocking` asserts exactly that). Rendering, on macOS,
   has to happen where AppKit is. That means the worker becomes a queue and
   the render itself is dispatched to the main queue — which only works if the
   embedding process is running a main run loop, so the ABI would grow that
   requirement and the two platforms would stop behaving the same way.
2. **Declare the async path macOS-unsupported for now** and make
   `gpui_render_submit_async` return `RendererUnavailable` there rather than
   panicking into `-6`, so the failure is a documented answer instead of a
   caught panic. This is cheap and honest, and it costs the cheap per-frame
   lane every later milestone was told it could use on this platform; the two
   `#[cfg(target_os = "macos")]` assertions would have to be rewritten, which
   is a weakening and must be recorded as one.

Either way, `-6` reaching a caller as *"a panic happened somewhere"* is itself
worth removing: `worker_main`'s `catch_unwind` converts a precise upstream
precondition failure into the least informative code the enum has.

## Related

- `CodeTracer-Platform.milestones.org` PLAT-37, whose *"THE PIXEL PATH CHOSEN
  BY MEASUREMENT"* deliverable names `gpui_render_submit_async` /
  `gpui_render_try_take` / `gpui_render_cancel` as part of the headless path it
  prices, and whose 2026-10-02 status note measures the SYNCHRONOUS half of
  that path as working on macOS.
- `codetracer-specs/issues/2026-09-29-gpui-window-capture-lanes-are-wayland-only.md`
  — the headless path is that issue's proposed macOS pixel route; this defect
  bounds it to the synchronous entry point.
