# `letter-spacing` is parsed, stored and reported, but never applied to a drawn element

| | |
|---|---|
| Status | blocked (m1:fix-letter-spacing-is-parsed-and-reported-bu): owner decision required; see below |
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

## Blocker — 2026-10-18

**Still reproduces.** Measured 2026-10-18 on aarch64-darwin against
`isonim-gpui@95a61f8` (the tip of `agents`): a search of
`rust/gpui-nim-shim/src/` for `letter_spacing|letter-spacing` returns the
same three sites (`lib.rs:1454` dump, `render_sync.rs:98` field,
`render_sync.rs:203` parser arm) plus a comment at `render_sync.rs:411`;
nothing consumes the field. `gpui-pre-0.3.5/src` still has no
letter-spacing (every case-insensitive hit for
`letter_spacing|letter.spacing|tracking` is unrelated "tracking" prose;
control: `line_height` is found in 9 files).

**Not fixed, because the fix is the choice this issue calls a product
decision.** Both suggested directions satisfy the proposed rule ("take
effect or be rejected where it is parsed"), and no specification picks
between them: `docs/shim-design.md` only says an unknown property is
ignored, and nothing in `isonim-pm/spec/` (`isonim-gpui-window-decorations.md`,
`isonim-cross-platform-architecture.md`, `IsoNim.md`) states which CSS
properties the GPUI renderer must draw. What each option costs, so the
owner can decide:

1. **Drop it** from the parser arm, the `GpuiStyles` field and the dump
   (and the `letter_spacing: None` literal in `tests/gpui_rendering.rs`).
   No pixel changes, since nothing draws it today. But the IsoNim editor's
   own chrome sets it (`isonim/src/isonim/editor/views/gallery_overlay.nim`,
   `inspector_sections.nim`), and the editor's property-evidence table
   (`viewmodels.nim`: `propertyEvidenceRow("typography", @["letter-spacing"], ltpesValidated, …)`)
   claims letter-spacing as validated; after the drop a GPUI render plan
   would silently stop carrying a property that table claims.
2. **Implement it.** `gpui-pre` is consumed from crates.io at `=0.3.5`
   (`docs/gpui-pin.md`), so it cannot be patched in place; the shim would
   have to lay glyphs out itself (its own shaping and wrapping instead of
   `StyledText`), and the fused-run path in `inline_text_runs` would have
   to fold the spacing into per-run widths. That is a feature, not a
   defect-fix.

Decision needed: one of the two, or a third (for example, keep parsing but
mark the property as unapplied in the dump). No code change was made.

## Related

- `2026-10-17-every-published-gpui-pre-ceils-a-text-elements-width.md`
