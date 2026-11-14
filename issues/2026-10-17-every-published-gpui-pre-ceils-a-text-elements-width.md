# Every published gpui-pre rounds a text element's width up to a whole pixel, so one element per span accumulates error along a row

| | |
|---|---|
| Status | open (worked around in the shim; the upstream rounding is unchanged) |
| Recorded | 2026-10-17 |
| Observed in | upstream `gpui-pre` 0.3.5 – 0.3.9; shim measured in `isonim-gpui` @ `779f0d1` |
| Area | `gpui-pre` `src/elements/text.rs`, `TextLayout::layout`; consumed by `rust/gpui-nim-shim` |

## Observed

A `gpui` text element ceils its own measured width:

```rust
size.width = size.width.max(line_size.width).ceil();
```

That line is present in **every published version of the crate**, at
`src/elements/text.rs:765` in 0.3.5 and 0.3.6 and at `:772` in 0.3.7, 0.3.8
and 0.3.9 (the file grows from 1316 to 1349 lines across that boundary,
which accounts for the +7).

Drawing a syntax-highlighted row as one text element per coloured span
therefore applies the rounding once per span, and each span begins at the
inflated running total of the ones before it. The error accumulates left to
right. Measured on aarch64-darwin against a browser rendering of the same
text, the per-row best-fit shift grew to +6 device pixels across one window,
with per-row slopes taking the discrete values 0.000 / 0.278 / 0.833 / 1.071 /
1.290 / 1.667 % — the ladder an integer number of spans per row predicts.

## Expected

Not specified upstream. Proposed: a text element should be able to report an
unrounded width, so a caller laying out several runs on one line can round
once, as a browser does — it accumulates inline runs at 1/64 px and rounds at
the end.

## Evidence

Each version's crate was fetched to a scratch directory and read there; the
cargo registry was not written to (it still holds 0.3.5 only, and the 0.3.5
in it is byte-identical to the fetched copy, `diff -q` rc=0). Controls, all
run per version:

- **Non-ceil variant control.** `grep -c 'size.width.max(line_size.width);'`
  (the same statement without `.ceil()`) returns 0 in all five versions, so
  the match is not a substring that would hit either form.
- **Absent-sentinel control.** A string known not to be present exits 1.
- **Present-sentinel control.** `TextLayoutInner` exits 0, so the files are
  really being read.

`examples/text.rs` also exists in each crate; its only `ceil` is an unrelated
`div_ceil`, so the line numbers above are unambiguous.

## Suggested direction

The shim now avoids the accumulation for rows that declare themselves an
inline formatting context, by drawing their text-only children as one
`StyledText::with_highlights` (`inline_text_runs` in `render_sync.rs`). That
removes the per-span rounding without touching the crate, and it is why the
pin stays at `=0.3.5`: no newer version fixes this, so there is nothing to
upgrade toward.

What the workaround does not cover is a row whose spans are not text-only —
an inline-value chip is a real box and correctly declines to fuse, and the
text inside it still ceils per span.

Upgrading the pin is not a remedy for this and should not be proposed as one.
An upstream change, or a fork, is what would remove the rounding itself.

## Related

- `rust/gpui-nim-shim/src/render_sync.rs`, `inline_text_runs`
- `tools/check_gpui_pin.sh` holds the pin at `=0.3.5`
