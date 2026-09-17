# The GPUI pin, and how to move it

**Pinned version:** `gpui-pre 0.3.5` — a snapshot of
`zed-industries/zed@d89e9c2124b2786a390c7a451c7488601b4da2e1` (2026-09-14).

GPUI is developed inside Zed for Zed. It has no stable public API and no
semantic-versioning contract, and the crates.io `gpui` releases published
by zed-industries lag the tree we need badly: the newest is **0.2.2**, from
**2025-10-22**, which predates `HeadlessAppContext` and
`Window::render_to_image` entirely.

Until 2026-09-17 this repository closed that gap with a **git dependency
pinned by commit** (`zed-industries/zed@562a0e03`, 2026-04-28). It now
consumes the **`gpui-pre` family on crates.io** instead, pinned to an exact
version. This document says why, because the choice has a cost as well as a
benefit and both should be visible to whoever moves the pin next.

## Why `gpui-pre` rather than the git pin

Type identity in Rust is per **package**, not per lib name. Two crates
compiled from byte-identical source under two different package names
produce two incompatible `App`s, two incompatible `Entity<T>`s and two
incompatible `Window`s, and nothing in the resulting error message says so
in those words.

`gpui-kit` (`gpui-component`, `gpui-base` — the dock, the tab group, the
code editor) depends on `gpui-pre`. While this repository depended on
`gpui` from the zed git remote, **no gpui-kit component could be handed
anything this shim builds**, no matter how close the two revisions were.
That is the skew that made the dock and editor work unreachable; it was a
package-name mismatch, not an API gap.

`gpui-pre` also removes the git-checkout build cost (`cargo` no longer
clones Zed's monorepo to build one crate), and the family carries the
source revision in each crate's `[package.metadata.gpui-pre]`, so the
provenance is still recorded.

## What it costs

**`gpui-pre` is not published by zed-industries.** Its crates.io owner is
`huacnlee` (Jason Lee), who maintains `longbridge/gpui-kit`. The crates are
an Apache-2.0 republication of Zed's tree, complete and unmodified as far as
the source shows — `gpui-pre`'s own `Cargo.toml.orig` still reads
`zed-crate = "gpui"`, `zed-rev = "d89e9c21…"` — but the release cadence,
and whether there is a release at all, is a third party's decision rather
than upstream's.

The practical consequences:

- A revision we need may not be published. There is no `cargo`-level escape
  hatch back to a git revision that keeps gpui-kit working, because that is
  the mismatch this pin exists to remove.
- The republication lags upstream by days, not months, today: 0.3.5 is
  `zed@d89e9c2` (2026-09-14) and Zed's `main` was `ccadf399` (2026-09-17)
  when this was written.

This is a deliberate trade: reaching gpui-kit at all requires sharing its
package graph.

## What the gate asserts

`tools/check_gpui_pin.sh` is the gate. It is not a style check: the version
is written in more than one place and cargo will silently resolve a
`gpui_platform` built against one `gpui` next to another.

1. Every `gpui-pre*` dependency under `rust/**/Cargo.toml` requests the
   **same exact version** — and *every one of them requests a version at
   all*, which is an equality rather than a floor, because a caret
   requirement parses to nothing and would otherwise drop silently out of a
   set the gate then finds unanimous.
2. Every `gpui-pre*` package resolved in `rust/Cargo.lock` is at that same
   version — so a manifest bumped without `cargo update` is red rather than
   quietly building the old tree.
3. **No second GPUI core is resolved.** A `zed-industries/zed` git source,
   or the crates.io `gpui` package, appearing alongside `gpui-pre` is the
   two-incompatible-`App`s state described above, and it is exactly the
   state the repository was in before this pin moved.
4. The version named at the top of **this file** is that same string — so
   the document a reader trusts cannot fall behind the build.

Checks 1, 2 and 4 carry non-vacuity floors. A scan that matched nothing
would satisfy "all of them agree" for free, which is the one way a gate like
this fails without anybody noticing (traps 4 and 6a in
`codetracer-specs/Testing/Verification-Harness-Traps.md`). Check 3 asserts an
**absence**, so an empty scan is its passing answer and a floor there would
mean nothing. That its file is read at all is inherited from checks 1 and 2.
That it can fire is not inherited: both of its branches — a zed git source,
and the crates.io `gpui` package — are planted by their own mutation arm and
have been watched go red.

## Where the pin is written

| file | what it pins |
| --- | --- |
| `rust/gpui-nim-shim/Cargo.toml` — `[dependencies]` | `gpui` = `gpui-pre`, `gpui_platform` = `gpui-pre-platform` (both optional) |
| `rust/gpui-nim-shim/Cargo.toml` — `[dev-dependencies]` | `gpui` = `gpui-pre` with `test-support`, for the `TestAppContext` integration tests |
| `rust/Cargo.lock` | what cargo actually resolved |
| this document | what a reader is told |

The dependency **keys** are still `gpui` and `gpui_platform`. They are
`package = ` renames, and they are what keeps `src/` unchanged: `gpui-pre`'s
`[lib] name` is already `gpui` and `gpui-pre-platform`'s is already
`gpui_platform`, so no `use` statement and no `gpui_platform/x11`-style
feature spelling had to move.

## Updating the pin

The pin should move for a reason — a fix or an API we need — not on a
schedule. The procedure is:

```bash
# 0. Know why. Record the reason in the commit message and the PR.
NEW=<version published on crates.io as gpui-pre>

# 1. Move every manifest pin together. There is no way to interpolate a
#    version requirement in a cargo manifest, so this is a literal edit in
#    each place and the gate in step 3 is what makes it safe.
#
#    The manifest sed is anchored to `version = "=…"`, and the document sed
#    to the HEADLINE, deliberately: a blanket `s/0\.3\.5/$NEW/g` also
#    rewrites the prose below, including the measured toolchain floor and
#    the recorded zed revision, turning true statements about 0.3.5 into
#    false ones about $NEW.
sed -i "s/version = \"=0\.3\.5\"/version = \"=$NEW\"/g" rust/gpui-nim-shim/Cargo.toml
sed -i "s/^\*\*Pinned version:\*\* \`gpui-pre 0\.3\.5\`/**Pinned version:** \`gpui-pre $NEW\`/" docs/gpui-pin.md
# …then update the rest of this document BY HAND: the zed revision and date
# in the headline, the toolchain floor, and the two "at 0.3.5" sections.

# 2. Re-resolve, so the lock records the new version rather than the old.
cd rust && cargo update && cd ..

# 3. The gate. Must be green BEFORE anything else is believed.
./tools/check_gpui_pin.sh

# 4. Both featured builds. These are the two that actually touch GPUI's
#    API surface; the default build does not and will pass over a pin
#    that has broken everything.
cd rust
cargo build --features gpui-backend      # the windowed launch path
cargo build --features gpui-headless     # the off-screen RGBA path
cargo test  --features gpui-backend      # TestAppContext integration tests
cargo test  --features gpui-headless     # pixel-capture integration tests
cd ..

# 5. The windowed pixel lane. Nothing above looks at a pixel: every step in
#    4 passes against a window that renders nothing at all, which is what
#    the Xvfb path was measured doing.
just test-gui

# 6. The rest of the suite, through the lane, so the case count is
#    reported and a suite that stopped running is red rather than absent.
./ci/run-suite.sh --with-gpui
```

**Step 4 is the step that cannot be skipped.** `cargo test` with default
features compiles none of GPUI: `gpui` is an *optional* dependency and
the three integration test files are each `#![cfg(feature = "…")]`, so a
default-feature run reports `running 0 tests` for all three and is green
over a pin that does not build at all. That was measured on 2026-09-15
and is the reason `ci/run-suite.sh` names the excluded subset out loud
rather than reporting a total that quietly omits it.

## Toolchain floor

**`gpui-pre` 0.3.5 does not build on Rust 1.94.1.** `src/profiler.rs` calls
`std::hint::cold_path()` twice; it was stabilised in **1.95.0**, and on the
older compiler the build ends in two `E0658: use of unstable library
feature 'cold_path'` errors — after ~1,000 dependency crates have compiled
fine, so the failure arrives late and says nothing about GPUI.

`flake.nix` takes `fenixPkgs.stable`, so the floor is enforced by
`flake.lock`. It was moved from fenix `380f1969` (2026-04-01, rustc 1.94.1)
to `f8ac2cd5` (2026-09-17, **rustc 1.98.1**) for this reason. `nixpkgs` was
deliberately left where it was: nothing else in the shell needed to move.

## Known platform asymmetry at this version

`gpui_platform::current_headless_renderer()` still returns `None` on Linux
at 0.3.5 — it is `Some(MetalHeadlessRenderer)` under
`#[cfg(target_os = "macos")]` and `None` otherwise — so
`gpui_render_to_pixels` answers `RendererUnavailable` (error code 2) there
and the off-screen pixel-capture assertions run on macOS only. Linux gets
its pixels from the windowed lane (`just test-gui`, headless sway + `grim`)
instead. This is stated in
`rust/gpui-nim-shim/tests/test_headless_render.rs`'s header and asserted by
the test rather than being worked around. A pin bump is the right moment to
re-check whether it still holds: if Linux gains a headless renderer, the
Linux arm of that test should stop being the error-code arm.

## Shutdown internals this repository depends on

`gpui_quit` / `gpui_quit_after_ms` / `gpui_quit_requested` (see
`rust/gpui-nim-shim/src/window.rs`) are built on GPUI internals that have no
API contract. Re-read them on every pin bump. At 0.3.5 all three still hold:

- `App::quit()` → `Platform::quit()`, and on Linux that is
  `self.inner.with_common(|common| common.signal.stop())` — the calloop
  `LoopSignal` (`gpui-pre-linux-0.3.5/src/linux/platform.rs:335`).
- `QuitMode::Explicit` still exists, and `QuitMode::Default` is still
  `LastWindowClosed` off macOS — so `launch_gpui_app` still has to set
  `Explicit` itself.
- `Drop for WaylandWindow` still ends by `spawn`-ing `state_ptr.close()` +
  `client.drop_window(&surface_id)` on the **foreground** executor and
  detaching it (`…/src/linux/wayland/window.rs:757`). The 150 ms drain
  between window removal and `cx.quit()` in `gpui_app.rs` is still
  load-bearing for that reason.

## Related decisions

- **`gpui-kit` is still not a dependency of this workspace**, but it is now
  *reachable*: it depends on `gpui-pre ^0.3.1`, which `=0.3.5` satisfies, so
  adding `gpui-component` would unify with the graph this shim builds rather
  than duplicating it. Adding it is a separate milestone; establishing that
  it resolves is not the same as building a binding to it.
- PLAT-4 read gpui-kit's `Tiles` implementation upstream on `main` at
  `9796bb7c` and **declined** it — a `TilePanel` is
  `{ panel, bounds: Bounds<Pixels>, z_index }`, i.e. free positioning with
  overlap, which the layout spec declares a non-goal. **Upstream has since
  agreed**: `longbridge/gpui-kit@3f1dda6` ("dock: Remove the tiles canvas",
  2026-09-11, 67 files, −5,069 lines) deleted
  `crates/base/src/dock/tiles_state.rs`, `tiles_geometry.rs` and
  `crates/component/src/dock/tiles.rs`. Note that the published **0.6.1**
  crates (2026-09-09) predate that commit by two days and *still carry*
  tiles; a dependency added at 0.6.1 would get it, and would lose it at the
  next release.
- Zed's own `PaneGroup` has no `Tiles` equivalent, so the pinned `gpui`
  revision does not offer that model either way.
