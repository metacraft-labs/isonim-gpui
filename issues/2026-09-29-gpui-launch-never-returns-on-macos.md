# `gpui_launch` never returns on macOS: the shutdown path terminates the process instead of unwinding the FFI call

| | |
|---|---|
| Status | open |
| Recorded | 2026-09-29 |
| Observed in | isonim-gpui @ `40695f9`, shim built `--features gpui-backend`, aarch64-darwin (macOS 15) |
| Area | `rust/gpui-nim-shim/src/gpui_app.rs` `launch_gpui_app` / `spawn_shutdown_poller`; `rust/gpui-nim-shim/src/window.rs` `gpui_quit` / `gpui_quit_after_ms` |
| Found by | first run of the GPUI front-end on a macOS host (the campaign that built it ran on Linux only) |

**This repository has no `issues/` folder before this file, so there is no
archive to search here.** `git log -i -S'gpui_launch' -- issues/` and
`git log --all -- 'issues/*'` both return empty at `40695f9`. The consumer-side
archive was searched instead (`codetracer-specs/issues/`, pickaxe on
`gpui_launch`, `wayland`, `macos`) and holds nothing on this.

## Observed

A minimal probe — build the shim with `--features gpui-backend`, arm the
auto-quit deadline, call `gpui_launch`, print on both sides of it:

```nim
echo "BEFORE gpui_launch"
gpui_quit_after_ms(750)
let t0 = epochTime()
gpui_launch("launch probe".cstring, 400.0, 300.0, builder)
echo "AFTER gpui_launch  elapsed_ms=", int((epochTime() - t0) * 1000)
echo "REACHED-END"
```

prints, in full:

```
BEFORE gpui_launch
BUILDER-RAN root=true
```

and the process exits with status **0**. `AFTER gpui_launch` and `REACHED-END`
never appear. The event loop is entered, the root builder runs, the auto-quit
deadline fires — and then the process is gone. `gpui_launch` does not return
on this platform.

The same shape takes the repository's own suite down silently. `tests/test_gui`
built with `-d:gpuiBackend` against the windowed shim prints

```
[Suite] GUI - Render Plan Smoke Tests
  [OK] create_element_and_verify_render_plan
  … (nine [OK] lines) …
  [OK] no_handlers_by_default

[Suite] GUI - Launch Integration Tests
```

and stops there, **exit code 0, 1.18 s wall**. The five `gpui_launch_*` cases
and the `GUI - GPUI Backend Compile Check` suite after them never run and never
report. Nothing in the output says a case was skipped; a caller reading the
exit status sees a pass.

## Expected

`gpui_app.rs`'s own contract, stated in `launch_gpui_app` immediately after the
`Application::run` call:

> ```rust
> // The event loop has returned -- the user closed the window, or a
> // quit was requested via `gpui_quit` / the auto-quit deadline.
> if window_id != 0 {
>     window::close_window(window_id);
> }
> ```

and `spawn_shutdown_poller`'s header: *"Three things end the loop, and all
three take the same route out — remove the windows, drain, quit."* Both
sentences assume control comes back. On macOS none of the code after
`Application::with_platform(...).run(...)` executes: the window is not closed
in the registry, `set_auto_quit_ms(0)` does not disarm, `clear_quit_request()`
does not run, and `ACTIVE_WINDOW_ID` is not reset.

`tests/test_gui.nim`'s Launch-Integration header states the same expectation
directly — *"what changes under `-d:gpuiBackend` is that a window is opened and
closed five times in a row"* — which is not reachable if the first launch ends
the process.

## Evidence

**Measured on this host**, aarch64-darwin, macOS 15, 2026-09-29:

```sh
cd isonim-gpui
nix develop --command bash -c 'cd rust && cargo build --features gpui-backend'
#   Finished `dev` profile … in 1m 56s
#   rust/target/debug/libgpui_nim_shim.dylib, 50,000,568 bytes
nix develop --command bash -c 'nim c -r --hints:off -d:gpuiBackend \
  --path:src --path:../isonim/src -o:/tmp/launch_probe /tmp/launch_probe.nim'
#   BEFORE gpui_launch
#   BUILDER-RAN root=true
#   (exit 0; no further output)

nix develop --command bash -c 'nim c -r --nimcache:nimcache/test_gui \
  -d:gpuiBackend --path:../isonim/src tests/test_gui.nim'
time (timeout 60 ./tests/test_gui >/dev/null 2>&1; echo "exit=$?")
#   exit=0
#   0.09s user 0.06s system 12% cpu 1.178 total
```

The Rust side builds cleanly on macOS, which is worth stating separately
because it was also unmeasured: the windowed profile compiles `objc2-app-kit`,
`gpui-pre-apple`, `gpui-pre-macos` and `accesskit_macos` with no errors and one
future-incompat warning (`block v0.1.6`). The featureless profile builds in
2m 12s. **Nothing about the macOS build is broken; only the shutdown path is.**

**Read from the pinned upstream source** (inferred cause, not measured by
instrumenting gpui itself). `gpui-pre-macos-0.3.5/src/platform.rs`:

```rust
    fn quit(&self) {
        …
        unsafe {
            DispatchQueue::main().exec_async_f(ptr::null_mut(), quit);
        }

        extern "C" fn quit(_: *mut c_void) {
            unsafe {
                let app = NSApplication::sharedApplication(nil);
                let _: () = msg_send![app, terminate: nil];
            }
        }
    }
```

`-[NSApplication terminate:]` ends the process; it does not return to
`app.run()`'s caller, so `MacPlatform::run`'s `pool.drain()` and everything
after it is dead code once a quit is issued. The Linux counterpart this
repository's `docs/gpui-pin.md` cites — *"`App::quit()` → `Platform::quit()`,
and on Linux that is `self.inner.with_common(|common| common.signal.stop())` —
the calloop `LoopSignal` (`gpui-pre-linux-0.3.5/src/linux/platform.rs:335`)"* —
makes the loop **return**. The asymmetry is between the two platform
implementations, and `docs/gpui-pin.md`'s "Shutdown internals this repository
depends on" section audits only the Linux one.

That section also already records that the two platforms differ here
(*"`QuitMode::Default` is still `LastWindowClosed` off macOS"*), so the
difference was known; what was not measured is that it makes the FFI call
one-way.

## Impact

- `tests/test_gui` is a **false green** under `-d:gpuiBackend` on macOS: seven
  cases across its last three suites are silently not run and the binary
  exits 0 — the five `gpui_launch_*` cases of *"GUI - Launch Integration
  Tests"*, `gpui_backend_feature_enabled`, and
  `window_paints_the_scene_the_shadow_tree_describes`. Only the nine cases of
  *"GUI - Render Plan Smoke Tests"*, which run before the first launch,
  report at all.
- Any consumer that does work after `gpui_launch` loses that work on macOS.
  The concrete one today is `codetracer`'s `src/frontend/gpui/main.nim`
  `launchWindow`, which writes `--frame-report=` and `--input-probe=` after
  the call; see `codetracer-specs/issues/2026-09-29-gpui-window-results-are-written-after-a-launch-that-never-returns.md`.

## Suggested direction

Three routes, with their trade-offs; none is obviously the one.

1. **Do the post-launch work before the loop, or from inside it.** Move
   `close_window` / `set_auto_quit_ms(0)` / `clear_quit_request()` into the
   shutdown poller, in the same update that calls `cx.quit()`, so they run on
   both platforms. Cheap and local, but it does not make `gpui_launch` return —
   consumers with their own post-launch work still lose it, so it fixes the
   shim's invariants and not the ABI's.
2. **Make the macOS quit path unwind instead of terminate.** Stop the run loop
   (`CFRunLoopStop` on the main loop, or `[NSApp stop:]` plus a posted event)
   rather than reaching `Platform::quit`. This makes `gpui_launch` a normal
   blocking call on both platforms, which is what every caller already assumes
   — but it leans on AppKit behaviour upstream does not contract, and
   `docs/gpui-pin.md`'s rule (*"re-read them on every pin bump"*) would grow a
   third entry.
3. **Declare the one-way call in the ABI and make the suite prove it.** If
   terminating is accepted as the macOS contract, `tests/test_gui` must not be
   able to exit 0 after skipping its remaining cases — a `quit`-time hook that
   fails the process unless every case has reported would turn today's silent
   pass into a loud failure. This documents the defect rather than removing it,
   and leaves every consumer to work around it.

Whichever is chosen, the regression that must not survive it is the silent
exit-0: a suite that stops in the middle and reports success is what made this
invisible to a Linux-only campaign.
