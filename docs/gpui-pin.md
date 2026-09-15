# The GPUI pin, and how to move it

**Pinned revision:** `zed-industries/zed@562a0e03b5c3c6e696154de92fd56302a41683c4`

GPUI is developed inside Zed for Zed. It has no stable public API, no
semantic-versioning contract, and the crates.io `gpui` releases lag the
tree we need (`HeadlessAppContext`, `Window::render_to_image`). So this
repository consumes it as a **git dependency pinned by commit**, and the
pin is a decision that has to be made deliberately rather than drifted
into.

`tools/check_gpui_pin.sh` is the gate. It is not a style check: it exists
because the revision is written in more than one place and cargo will
silently resolve two Zed checkouts if two of them disagree.

## What the gate asserts

1. Every `rev = "…"` under `rust/**/Cargo.toml` is the **same** string.
2. Every `zed-industries/zed?rev=` line resolved in `rust/Cargo.lock` is
   that same string — so a manifest bumped without `cargo update` is red
   rather than quietly building the old tree.
3. The revision named at the top of **this file** is that same string —
   so the document a reader trusts cannot fall behind the build.

Each check carries a non-vacuity floor. A scan that matched nothing would
satisfy "all of them agree" for free, which is the one way a gate like
this fails without anybody noticing (traps 4 and 6a in
`codetracer-specs/Testing/Verification-Harness-Traps.md`).

## Where the pin is written

| file | what it pins |
| --- | --- |
| `rust/gpui-nim-shim/Cargo.toml` — `[dependencies]` | `gpui`, `gpui_platform` (both optional) |
| `rust/gpui-nim-shim/Cargo.toml` — `[dev-dependencies]` | `gpui` with `test-support`, for the `TestAppContext` integration tests |
| `rust/Cargo.lock` | what cargo actually resolved |
| this document | what a reader is told |

## Updating the pin

The pin should move for a reason — a fix or an API we need — not on a
schedule. The procedure is:

```bash
# 0. Know why. Record the reason in the commit message and the PR.
NEW=<40-hex commit from zed-industries/zed>

# 1. Move every manifest pin together. There is no way to interpolate a
#    git rev in a cargo manifest, so this is a literal edit in each place
#    and the gate in step 3 is what makes it safe.
sed -i "s/562a0e03b5c3c6e696154de92fd56302a41683c4/$NEW/g" \
    rust/gpui-nim-shim/Cargo.toml docs/gpui-pin.md

# 2. Re-resolve, so the lock records the new revision rather than the old.
cd rust && cargo update -p gpui -p gpui_platform && cd ..

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

# 5. The rest of the suite, through the lane, so the case count is
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

## Known platform asymmetry at this revision

`gpui_platform::current_headless_renderer()` returns `None` on Linux at
`562a0e03`, so `gpui_render_to_pixels` answers `RendererUnavailable`
(error code 2) there and the pixel-capture assertions run on macOS only.
This is stated in `rust/gpui-nim-shim/tests/test_headless_render.rs`'s
header and asserted by the test rather than being worked around. A pin
bump is the right moment to re-check whether it still holds: if Linux
gains a headless renderer, the Linux arm of that test should stop being
the error-code arm.

## Related decisions

- **`gpui-kit` is not a dependency of this workspace.** PLAT-4 read its
  `Tiles` implementation upstream on `main` at `9796bb7c` and **declined
  it** — a `TilePanel` is `{ panel, bounds: Bounds<Pixels>, z_index }`,
  i.e. free positioning with overlap, which the layout spec declares a
  non-goal. Nothing in this repository pins `gpui-kit`, so PLAT-19's
  pinning work does not reach it and the `9796bb7c` reading was not
  re-taken here. If a later milestone adds `gpui-kit` as a dependency,
  it inherits this document's procedure and owes a re-read of
  `crates/base/src/dock/tiles_state.rs` at whatever revision it pins.
- Zed's own `PaneGroup` has no `Tiles` equivalent, so the pinned `gpui`
  revision does not offer that model either way.
