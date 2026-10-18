# `min-height`, `max-width` and `max-height` are parsed and reported, but never applied

| | |
|---|---|
| Status | open |
| Recorded | 2026-10-18 |
| Observed in | `isonim-gpui` @ `1794cc5` (branch `agents`) |
| Area | `rust/gpui-nim-shim/src/render_sync.rs` (`GpuiStyles`, the CSS property table), `src/lib.rs` (the style dump), `src/gpui_app.rs` (the element builder) |

Found while reviewing the fix for the `letter-spacing` issue of the same
shape (fixed in `1794cc5`), which added the rule this defect breaks.

## Observed

At `1794cc5` the three properties are parsed and dumped, but nothing
applies them when an element is built:

```
rust/gpui-nim-shim/src/render_sync.rs:154: "min-height" | "min_height" | "min_h" => self.min_h = Some(v),
rust/gpui-nim-shim/src/render_sync.rs:155: "max-width" | "max_width" | "max_w" => self.max_w = Some(v),
rust/gpui-nim-shim/src/render_sync.rs:156: "max-height" | "max_height" | "max_h" => self.max_h = Some(v),
rust/gpui-nim-shim/src/lib.rs:1428-1430:  push_style!(min_h / max_w / max_h, …)
rust/gpui-nim-shim/src/gpui_app.rs:1438:  if let Some(ref mw) = styles.min_w {   # the only one of the four applied
```

A search of `gpui_app.rs` for `min_h`, `max_w` and `max_h` returns nothing.
`min_w` is the positive control: it is parsed, dumped and applied (`el.min_w(px(…))`).

## Expected

`docs/shim-design.md` § "Only Drawn Properties Are Parsed" (added in
`1794cc5`): a property is parsed into `GpuiStyles` and reported in the dump
only if the shim applies it when it builds the drawn element. That section
lists these three as known exceptions that predate the rule.

## Suggested direction

Unlike `letter-spacing`, GPUI's `Styled` has `min_h`, `max_w` and `max_h` (generated from the `min_size` / `max_size` prefixes in `gpui-pre-macros` `styles.rs`),
so applying them next to `min_w` in `gpui_app.rs` is the likely fix, plus a
render-plan or pixel test per property. Then remove them from the
exception list in `docs/shim-design.md`.

## Related

- `docs/shim-design.md` § "Only Drawn Properties Are Parsed"
- `font-style`, the remaining exception, guarded by `inline_runs_decline_a_span_with_a_font_style`
