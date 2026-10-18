# `letter-spacing` is parsed, stored and reported, but never applied to a drawn element

| | |
|---|---|
| Status | in-progress (m1:fix-letter-spacing-is-parsed-and-reported-bu) |
| Recorded | 2026-10-17 |
| Observed in | `isonim-gpui` @ `779f0d1` (branch `agents`) |
| Area | `rust/gpui-nim-shim/src/render_sync.rs` (`GpuiStyles`, the CSS property table), `src/lib.rs` (the style dump) |

## Observed

The property is accepted at every stage except the one that would make it
visible:

- `render_sync.rs`, the CSS property table: `"letter-spacing" | "letter_spacing" => self.letter_spacing = Some(v)`
- `render_sync.rs`, `GpuiStyles`: `pub letter_spacing: Option<String>`
- `lib.rs`, the style dump: `push_style!(letter_spacing, "letter_spacing")`

There is no fourth site. A search over the whole shim source
(`rust/gpui-nim-shim/src/`, including `gpui_app.rs`, which builds the drawn
elements) returns exactly those three hits, so nothing consumes the value
when an element is built.

The upstream crate has no such property either: a search of
`gpui-pre-0.3.5/src` for `letter_spacing` returns nothing. Positive control
for that search — `line_height`, which must exist — is found in
`text_system.rs`, `window.rs` and `styled.rs`.

The effect is that a stylesheet setting `letter-spacing` is silently
ineffective, while a render-plan reader and the style dump both report it as
set. A test asserting on the plan would pass; the pixels would not change.

## Expected

Not specified. Proposed: a declared property should either take effect or be
rejected where it is parsed. Reporting it as applied when nothing applies it
is the failure mode that costs the most, because it makes the plan disagree
with the window.

## Evidence

Measured 2026-10-17 on aarch64-darwin, against the commit above:

```
rust/gpui-nim-shim/src/lib.rs:1446:        push_style!(letter_spacing, "letter_spacing");
rust/gpui-nim-shim/src/render_sync.rs:98:    pub letter_spacing: Option<String>,
rust/gpui-nim-shim/src/render_sync.rs:203:            "letter-spacing" | "letter_spacing" => self.letter_spacing = Some(v),
```

Informational, found while investigating per-glyph advance differences
between this renderer and a browser. It is **not** a contributor to that
difference: neither arm's measured text sets the property.

## Suggested direction

Two options, and the choice is a product decision rather than a clear
defect-fix:

1. Drop the property from the parser and the dump, so nothing claims support
   that does not exist. Cheapest, and honest.
2. Implement it. `gpui-pre` offers no letter-spacing, so this means spacing
   runs by hand at the shim level, which interacts with the row fusion added
   in `inline_text_runs` — fused runs would need the spacing folded into
   their per-run widths.

## Related

- `2026-10-17-every-published-gpui-pre-ceils-a-text-elements-width.md`
