//! Render-sync bridge: translates the shadow element tree into GPUI's declarative model.
//!
//! This module is the core of the imperative->declarative bridge. It walks the
//! shadow tree maintained by the C FFI layer and produces a render plan describing
//! what GPUI elements to create.
//!
//! ## Mapping
//!
//! | Shadow tag      | GPUI element |
//! |-----------------|--------------|
//! | `div`, `section`, `header`, etc. | `div()` |
//! | `span`, `p`, `h1`-`h6`, etc.    | `div()` + text child (TextContainer) |
//! | `button`        | `div()` (with click handler) |
//! | `img`           | `img()` |
//! | `svg`           | `svg()` |
//! | `root`          | `div()` (full-size container) |
//! | text node       | string child of parent div |
//! | unknown tag     | `div()` (fallback container) |
//!
//! ## Style mapping
//!
//! Shadow style properties use CSS-like names. This module maps them to
//! GPUI's Tailwind-inspired builder methods:
//! - `background-color` -> `.bg()`
//! - `width` / `height` -> `.w()` / `.h()`
//! - `padding` / `margin` -> `.p()` / `.m()`
//! - `color` -> `.text_color()`
//! - `font-size` -> `.text_size()` (custom px)
//! - `border-radius` -> `.rounded()`
//! - `flex-direction` -> `.flex_row()` / `.flex_col()`
//! - `align-items` -> `.items_center()` etc.
//! - `justify-content` -> `.justify_center()` etc.
//! - `gap` -> `.gap()`

use std::collections::BTreeMap;

use crate::tree::{GpuiElementKind, Node, NodeId, NodeKind, Tree};

/// Determines what GPUI element a shadow node should map to.
pub fn classify_node(node: &Node) -> GpuiElementKind {
    match &node.kind {
        NodeKind::Text(_) => GpuiElementKind::TextNode,
        NodeKind::Element(tag) => classify_tag(tag),
    }
}

/// Maps a shadow tag name to a GPUI element kind.
/// This matches the `tag_to_gpui_kind` in `tree.rs` but is used by
/// the render-sync layer for consistency.
pub fn classify_tag(tag: &str) -> GpuiElementKind {
    crate::tree::tag_to_gpui_kind(tag)
}

/// A collected style ready to be applied to a GPUI element.
/// GPUI uses Tailwind-inspired builder methods; this struct normalizes
/// CSS-like property names to GPUI equivalents.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GpuiStyles {
    pub bg: Option<String>,
    pub w: Option<String>,
    pub h: Option<String>,
    pub min_w: Option<String>,
    pub min_h: Option<String>,
    pub max_w: Option<String>,
    pub max_h: Option<String>,
    pub p: Option<String>,
    pub m: Option<String>,
    pub flex_direction: Option<String>,
    pub gap: Option<String>,
    pub text_size: Option<String>,
    pub text_color: Option<String>,
    pub rounded: Option<String>,
    /// Per-corner radii (`border-top-left-radius`, ...), each overriding
    /// `rounded` on its own corner.
    pub rounded_tl: Option<String>,
    pub rounded_tr: Option<String>,
    pub rounded_br: Option<String>,
    pub rounded_bl: Option<String>,
    pub items: Option<String>,
    pub justify: Option<String>,
    pub border_width: Option<String>,
    /// Per-side widths (`border-top-width`, ...), each overriding
    /// `border_width` on its own side.
    pub border_top_width: Option<String>,
    pub border_right_width: Option<String>,
    pub border_bottom_width: Option<String>,
    pub border_left_width: Option<String>,
    pub border_color: Option<String>,
    pub shadow: Option<String>,
    pub opacity: Option<String>,
    pub overflow: Option<String>,
    pub font_family: Option<String>,
    pub font_weight: Option<String>,
    pub font_style: Option<String>,
    /// `text-decoration`: `underline` underlines the element's text.
    pub text_decoration: Option<String>,
    pub line_height: Option<String>,
    pub cursor: Option<String>,
    pub display: Option<String>,
    pub position: Option<String>,
    pub white_space: Option<String>,
    pub text_overflow: Option<String>,
    pub flex_shrink: Option<String>,
    /// `flex-grow: 1` — the element takes the free space on its parent's
    /// main axis (a row's band that must reach the parent's right edge).
    pub flex_grow: Option<String>,
    /// Per-side padding (`padding-left`, ...), each overriding `p` on its
    /// own side.
    pub padding_top: Option<String>,
    pub padding_right: Option<String>,
    pub padding_bottom: Option<String>,
    pub padding_left: Option<String>,
    /// Per-side margins (`margin-top`, ...), each overriding `m` on its own
    /// side.
    pub margin_top: Option<String>,
    pub margin_right: Option<String>,
    pub margin_bottom: Option<String>,
    pub margin_left: Option<String>,
    /// The inset of a `position: absolute` element from its containing
    /// block's edges, in pixels.
    pub top: Option<String>,
    pub right: Option<String>,
    pub bottom: Option<String>,
    pub left: Option<String>,
}

impl GpuiStyles {
    /// Collect styles from a shadow node, normalizing CSS-like names to GPUI names.
    pub fn from_node(node: &Node) -> Self {
        let mut s = GpuiStyles::default();
        for (key, value) in &node.styles {
            s.apply(key, value);
        }
        // Also check attributes that might be styling (e.g. width/height set as attributes)
        for (key, value) in &node.attributes {
            // Skip internal attributes
            if key.starts_with("__") {
                continue;
            }
            // Only apply attribute-based styles if no explicit style was set
            s.apply_if_absent(key, value);
        }
        s
    }

    /// Apply a single CSS-like property.
    fn apply(&mut self, key: &str, value: &str) {
        let v = value.to_string();
        match key {
            "background" | "background-color" | "background_color" | "bg" => self.bg = Some(v),
            "width" | "w" => self.w = Some(v),
            "height" | "h" => self.h = Some(v),
            "min-width" | "min_width" | "min_w" => self.min_w = Some(v),
            "min-height" | "min_height" | "min_h" => self.min_h = Some(v),
            "max-width" | "max_width" | "max_w" => self.max_w = Some(v),
            "max-height" | "max_height" | "max_h" => self.max_h = Some(v),
            "padding" | "p" => self.p = Some(v),
            "margin" | "m" => self.m = Some(v),
            "flex-direction" | "flex_direction" => {
                self.flex_direction = Some(normalize_flex_direction(value))
            }
            "gap" => self.gap = Some(v),
            "font-size" | "font_size" | "text_size" => self.text_size = Some(v),
            "color" | "text-color" | "text_color" => self.text_color = Some(v),
            "border-radius" | "border_radius" | "rounded" | "corner-radius" | "corner_radius" => {
                self.rounded = Some(v)
            }
            "border-top-left-radius" | "border_top_left_radius" => self.rounded_tl = Some(v),
            "border-top-right-radius" | "border_top_right_radius" => self.rounded_tr = Some(v),
            "border-bottom-right-radius" | "border_bottom_right_radius" => {
                self.rounded_br = Some(v)
            }
            "border-bottom-left-radius" | "border_bottom_left_radius" => {
                self.rounded_bl = Some(v)
            }
            "align-items" | "align_items" | "items" => {
                self.items = Some(normalize_alignment(value))
            }
            "justify-content" | "justify_content" | "justify" => {
                self.justify = Some(normalize_justification(value))
            }
            "border-width" | "border_width" | "border" => self.border_width = Some(v),
            "border-top-width" | "border_top_width" => self.border_top_width = Some(v),
            "border-right-width" | "border_right_width" => self.border_right_width = Some(v),
            "border-bottom-width" | "border_bottom_width" => {
                self.border_bottom_width = Some(v)
            }
            "border-left-width" | "border_left_width" => self.border_left_width = Some(v),
            "top" => self.top = Some(v),
            "right" => self.right = Some(v),
            "bottom" => self.bottom = Some(v),
            "left" => self.left = Some(v),
            "border-color" | "border_color" => self.border_color = Some(v),
            "shadow" | "box-shadow" | "box_shadow" => self.shadow = Some(v),
            "opacity" => self.opacity = Some(v),
            "overflow" => self.overflow = Some(v),
            "font-family" | "font_family" => self.font_family = Some(v),
            "font-weight" | "font_weight" => self.font_weight = Some(v),
            "font-style" | "font_style" => self.font_style = Some(v),
            "text-decoration" | "text_decoration" => self.text_decoration = Some(v),
            "line-height" | "line_height" => self.line_height = Some(v),
            "cursor" => self.cursor = Some(v),
            "display" => self.display = Some(v),
            "position" => self.position = Some(v),
            "white-space" | "white_space" | "whitespace" => self.white_space = Some(v),
            "text-overflow" | "text_overflow" => self.text_overflow = Some(v),
            "flex-shrink" | "flex_shrink" => self.flex_shrink = Some(v),
            "flex-grow" | "flex_grow" => self.flex_grow = Some(v),
            "padding-top" | "padding_top" => self.padding_top = Some(v),
            "padding-right" | "padding_right" => self.padding_right = Some(v),
            "padding-bottom" | "padding_bottom" => self.padding_bottom = Some(v),
            "padding-left" | "padding_left" => self.padding_left = Some(v),
            "margin-top" | "margin_top" => self.margin_top = Some(v),
            "margin-right" | "margin_right" => self.margin_right = Some(v),
            "margin-bottom" | "margin_bottom" => self.margin_bottom = Some(v),
            "margin-left" | "margin_left" => self.margin_left = Some(v),
            _ => {} // Ignore unknown properties
        }
    }

    /// Apply a property only if the corresponding field is None.
    fn apply_if_absent(&mut self, key: &str, value: &str) {
        let already_set = match key {
            "width" | "w" => self.w.is_some(),
            "height" | "h" => self.h.is_some(),
            "padding" | "p" => self.p.is_some(),
            "margin" | "m" => self.m.is_some(),
            "background" | "background-color" | "background_color" | "bg" => self.bg.is_some(),
            "color" | "text-color" | "text_color" => self.text_color.is_some(),
            _ => false,
        };
        if !already_set {
            self.apply(key, value);
        }
    }
}

/// Normalize CSS flex-direction values to canonical names.
fn normalize_flex_direction(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "row" | "horizontal" => "row".to_string(),
        "column" | "col" | "vertical" => "column".to_string(),
        other => other.to_string(),
    }
}

/// Normalize CSS align-items values.
fn normalize_alignment(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "center" => "center".to_string(),
        "start" | "flex-start" => "start".to_string(),
        "end" | "flex-end" => "end".to_string(),
        "baseline" => "baseline".to_string(),
        "stretch" => "stretch".to_string(),
        other => other.to_string(),
    }
}

/// Normalize CSS justify-content values.
fn normalize_justification(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "center" => "center".to_string(),
        "start" | "flex-start" => "start".to_string(),
        "end" | "flex-end" => "end".to_string(),
        "space-between" | "space_between" => "space_between".to_string(),
        "space-around" | "space_around" => "space_around".to_string(),
        other => other.to_string(),
    }
}

/// Describes a single node in the render plan -- an intermediate representation
/// between the shadow tree and GPUI elements. This allows testing the mapping
/// logic without requiring actual GPUI rendering.
#[derive(Debug, Clone)]
pub struct RenderNode {
    /// The shadow node ID this render node corresponds to.
    pub node_id: u64,
    /// What kind of GPUI element to produce.
    pub kind: GpuiElementKind,
    /// The original HTML tag (for debugging/introspection).
    pub tag: String,
    /// Collected and normalized styles.
    pub styles: GpuiStyles,
    /// Text content (for text nodes or elements with __text_content).
    pub text: Option<String>,
    /// Whether this node has a "click" event listener.
    pub has_click_handler: bool,
    /// Whether this node has an "input" event listener (keyboard input).
    pub has_input_handler: bool,
    /// All event listener names attached to this node (for introspection/testing).
    pub event_names: Vec<String>,
    /// The element's PUBLIC attributes — everything not prefixed `__`.
    ///
    /// **Added because the plan could not see what the product writes.** The
    /// CodeTracer editor records its four debugger surfaces as attributes
    /// (`data-ct-pointer`, `data-ct-mark`, `data-ct-values`) and every one of
    /// them was absent from every captured plan: the element carried them and
    /// the introspection surface dropped them, so no test reading the plan
    /// could tell a row with an execution pointer from one without.
    ///
    /// All public attributes rather than a chosen few, so the plan does not
    /// answer only the questions somebody thought to ask when it was written.
    /// `__`-prefixed keys are internal (`__text_content` is already `text`).
    ///
    /// A `BTreeMap`, not the tree's `HashMap`: iteration order must be stable,
    /// or two plans of one tree serialise differently and every digest and
    /// byte-identity check built on the plan becomes a coin flip.
    pub attributes: BTreeMap<String, String>,
    /// Children render nodes (recursive).
    pub children: Vec<RenderNode>,
}

// ---------------------------------------------------------------------------
// Inline formatting contexts: many coloured spans, ONE text element
// ---------------------------------------------------------------------------

/// The attribute a container declares itself an INLINE FORMATTING CONTEXT
/// with. Its text-only children are then drawn as ONE piece of text carrying
/// per-span colours, instead of as one text element each.
///
/// WHY THIS EXISTS, AND WHY IT IS NOT A COSMETIC CHANGE. A GPUI text element
/// rounds its own width UP to a whole logical pixel —
/// `gpui-pre/src/elements/text.rs`, `size.width = size.width.max(line_size
/// .width).ceil()`, still present unchanged in every published version
/// through 0.3.9 (checked 2026-10-17). One element per span therefore
/// applies that rounding once PER SPAN, and because each span is laid out
/// after the ones before it, the error ACCUMULATES along the row: a row of
/// eleven spans can finish several device pixels right of where the same
/// text lands in a browser. Chromium accumulates inline runs in LayoutUnit
/// (1/64 px) and rounds once, at the end.
///
/// Fusing the spans into one element reproduces the browser's rule rather
/// than compensating for the difference: the width is rounded ONCE for the
/// whole context, so there is no per-span residue left to accumulate, at any
/// span count and any font size.
///
/// The declaration is explicit rather than inferred so that fusing is opt-in
/// per container: a caller that wants per-span elements (because it measures
/// them, or hangs something off them) keeps them by saying nothing.
///
/// WHAT FUSES. A child fuses when it is text and nothing else: no handler,
/// no window region, and no style beyond `color`, `font-weight` and
/// `flex-shrink` -- tested BY DIFFERENCE from the default style, so a style
/// field added later declines rather than fusing silently. One box property
/// is admitted on top of that: a span's own HORIZONTAL padding
/// (`padding-left` / `padding-right`, each a non-negative px length), and
/// only when this container itself declares `white-space: nowrap` (or `pre`)
/// and declares no `text-overflow`. A run carrying such padding is drawn by
/// the inset element (`gpui_app::InsetRunText`), which leaves EXACTLY the
/// padded gap between its shaped segments and rounds the whole run once; a
/// `StyledText` highlight cannot carry a gap, which is why padding never
/// fused before that element existed. The one-line condition is what keeps
/// it a pure layout change: the inset element draws one line and never
/// truncates, so it is only used where the unfused spans could not wrap or
/// elide either. `fusable_child_text` holds the full rule.
pub const INLINE_TEXT_RUNS_ATTRIBUTE: &str = "data-inline-text-runs";

/// One span of a fused inline formatting context: how many BYTES of the
/// fused string it covers, and the inline properties it is drawn with
/// (`None` on any of them keeps the inherited value).
///
/// The properties here are the ones the fused element can state per range
/// AND that the unfused path already draws, which is what bounds the set:
/// fusing must change where glyphs land and nothing else. `color` and
/// `font_weight` are stated by a `HighlightStyle`; the horizontal insets by
/// the inset element (`gpui_app::InsetRunText`), since a highlight cannot
/// carry a gap. See `fusable_child_text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineTextRun {
    /// The span's length in BYTES of the fused string, not characters:
    /// GPUI's highlight ranges are byte ranges.
    pub len: usize,
    /// The span's `color` style, verbatim; the caller parses it.
    pub color: Option<String>,
    /// The span's `font-weight`, verbatim; the caller parses it.
    pub font_weight: Option<String>,
    /// The span's own `padding-left`, verbatim, carried as a LEADING INSET:
    /// the gap the inset element leaves before the span's text. `None` when
    /// the span declares none. It is only ever `Some` in a context that
    /// promised one line, and only for a value `parse_inset_px` accepts (see
    /// `fusable_child_text`); the consumer parses it with that same function.
    /// Carried verbatim, like `color` and `font_weight`, so the plan type
    /// stays `Eq` and the one parser stays the single source of truth.
    pub inset_left: Option<String>,
    /// The span's own `padding-right`, verbatim: the TRAILING inset, the gap
    /// after the span's text. Carried under exactly the rules of
    /// `inset_left`.
    pub inset_right: Option<String>,
}

impl InlineTextRun {
    /// The leading inset in px, `0.0` when there is none.
    pub fn inset_left_px(&self) -> f32 {
        self.inset_left.as_deref().and_then(parse_inset_px).unwrap_or(0.0)
    }

    /// The trailing inset in px, `0.0` when there is none.
    pub fn inset_right_px(&self) -> f32 {
        self.inset_right.as_deref().and_then(parse_inset_px).unwrap_or(0.0)
    }

    /// Does this run carry a non-zero horizontal inset? A run that does
    /// cannot be drawn as a `StyledText` highlight, which has no notion of
    /// a gap, and needs the inset element (`gpui_app::InsetRunText`).
    pub fn has_inset(&self) -> bool {
        self.inset_left_px() > 0.0 || self.inset_right_px() > 0.0
    }
}

/// A span padding that can be reproduced EXACTLY as an inset: a finite,
/// non-negative length in px (`"7px"`, or a bare `"7"`).
///
/// The grammar is `gpui_app::parse_px`'s, character for character (trim,
/// strip one `px` suffix, parse the rest as `f32`), narrowed to finite
/// non-negative values. It must not be any LOOSER than that: a padding the
/// unfused path's `parse_px` rejects is not drawn at all, so an inset made
/// of it would add a gap the unfused span does not have. `"7 px"` is the
/// case to remember -- `parse_px` rejects it, so this does too.
///
/// Anything else -- `em`, `%`, `auto`, a negative or non-finite value -- is
/// `None`, and a span whose padding is `None` here keeps its own element.
/// That is deliberate: the unfused path either draws such a value in some
/// other way or ignores it, and the inset element must never guess which.
pub fn parse_inset_px(s: &str) -> Option<f32> {
    let s = s.trim();
    let num = s.strip_suffix("px").unwrap_or(s);
    num.parse::<f32>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
}

/// Does `context` PROMISE ONE LINE of text: does it declare
/// `white-space: nowrap` or `pre` (the two values the unfused path draws as
/// `whitespace_nowrap`, `gpui_app::apply_styles_to_div`) and declare no
/// `text-overflow` at all?
///
/// Only such a context may carry a span's horizontal padding as an inset.
/// The inset element shapes each segment as exactly ONE line and never
/// truncates, so in a context that may wrap or elide it would draw
/// something the unfused spans do not. In a context that promised one line
/// the unfused spans cannot wrap or elide either, so the inset element
/// changes where glyphs land and nothing else.
///
/// What it reads is the context's OWN declaration. A `text-overflow`
/// inherited from an ancestor is not in the plan node and is not seen here;
/// that is why the declaration is opt-in, and a caller that elides chips
/// must not declare them.
fn promises_one_line(context: &RenderNode) -> bool {
    let nowrap = context
        .styles
        .white_space
        .as_deref()
        .is_some_and(|ws| matches!(ws.trim(), "nowrap" | "pre"));
    nowrap && context.styles.text_overflow.is_none()
}

/// A child of the inline formatting context `context` that may be fused:
/// its text, or `None` if it must stay its own element.
///
/// A span qualifies only when it is text and NOTHING else — no handler, no
/// window region, no box styling. Anything else can be positioned, hit-tested
/// or painted in its own right, and fusing it away would change behaviour
/// rather than just layout. The one box property a span may carry is its
/// own HORIZONTAL padding, and only when `context` promised one line
/// (`promises_one_line`); `context` is passed for that check alone.
fn fusable_child_text<'a>(context: &RenderNode, child: &'a RenderNode) -> Option<&'a str> {
    if child.has_click_handler || child.has_input_handler || !child.event_names.is_empty() {
        return None;
    }
    if child
        .attributes
        .contains_key(crate::regions::REGION_ATTRIBUTE)
    {
        return None;
    }
    if !matches!(
        child.kind,
        GpuiElementKind::Div | GpuiElementKind::TextContainer
    ) {
        return None;
    }
    // INLINE-ONLY STYLING, TESTED BY DIFFERENCE FROM THE DEFAULT rather than
    // by listing the fields that are allowed to be absent. `GpuiStyles` has
    // some fifty fields and gains more; an allow-list would silently start
    // fusing away a property added later, which is the failure mode that is
    // hardest to see. Blanking the properties that a fused run CAN carry and
    // demanding the rest be untouched fails safe instead: a span with any
    // other style simply keeps its own element.
    //
    // WHAT BOUNDS THIS SET is a two-part test, not taste. Fusing replaces N
    // text elements with one fused element -- a `StyledText` plus one
    // highlight per range, or the inset element below -- so a property may
    // join the set only when (1) the fused element can state it per range
    // (for an inline property, a `HighlightStyle` can), and (2) the UNFUSED
    // path already draws it — otherwise
    // fusing would start drawing something the spans do not draw today, and
    // it would no longer be a pure layout change. `color` and `font-weight`
    // pass both; `flex_shrink` is meaningless once there is one element and
    // so is safe to ignore.
    //
    // `font-style` passes (1) and FAILS (2), which is why it is absent
    // although a `HighlightStyle` carries it. `GpuiStyles` parses and stores
    // `font-style` (`lib.rs` push_style) but nothing ever applies it: there
    // is no `italic` anywhere in this shim, so an unfused italic span is
    // drawn upright today. Fusing it would make the same markup render
    // italic only when it happens to fuse. The unapplied property is the
    // defect to fix, and fixing it is what would let `font-style` join this
    // set; `inline_runs_decline_a_span_with_a_font_style` is the guard until
    // then. (`letter-spacing` had the same shape -- parsed, reported, never
    // applied -- and was removed from the parser instead, because `gpui-pre`
    // has no letter-spacing to apply; see `docs/shim-design.md`.)
    //
    // NO BOX PROPERTY MAY BE CARRIED BY A HIGHLIGHT, and `padding_left` is
    // the one to think about, because it is how a caller spaces two text
    // runs inside a box. A `HighlightStyle` cannot carry it: fusing two runs
    // separated by padding into one `StyledText` CONCATENATES THEM AND
    // DELETES THE SPACE. Measured 2026-10-21 against codetracer 3fce565eb,
    // an editor inline-value chip separates its name from its value by
    // ~8.5 pt and BOTH front-ends draw that gap, so losing it would be an
    // 8.5 pt regression in exchange for removing one rounding of at most
    // 2 device px.
    //
    // HORIZONTAL PADDING NOW FUSES ANYWAY, because it is no longer carried
    // by a highlight. A span's own `padding-left` / `padding-right` rides on
    // its run as an INSET, and a run with an inset is drawn by the inset
    // element (`gpui_app::InsetRunText`), which leaves exactly that gap --
    // snapped to the device-pixel grid as taffy snaps the unfused span's
    // padding -- between the shaped segments. Under the two-part test above:
    // (1) the fused element can state it per range: yes, the inset element
    // does, which is what it exists for; (2) the unfused path draws it: yes,
    // `apply_styles_to_div` applies `padding_left` / `padding_right`.
    //
    // It fuses only under BOTH of these, and declines otherwise:
    //   * the CONTEXT promised one line (`promises_one_line`: it declares
    //     `white-space: nowrap` or `pre` and no `text-overflow`). The inset
    //     element draws one line and never truncates, so this is what keeps
    //     it a pure layout change; and
    //   * each padding is a finite non-negative px length (`parse_inset_px`).
    //     A value that does not parse must decline: the inset would have to
    //     guess what the unfused path makes of it.
    // Every OTHER box property still declines, and still by difference: the
    // `padding` shorthand (four sides), `padding-top` / `-bottom` (they
    // change the box's height), margins, backgrounds, and anything added
    // later. `inline_chunks_keep_a_padded_span_in_a_context_that_may_wrap`
    // and its neighbours are the guards.
    let mut probe = child.styles.clone();
    probe.text_color = None;
    probe.flex_shrink = None;
    probe.font_weight = None;
    let pads = [probe.padding_left.take(), probe.padding_right.take()];
    if probe != GpuiStyles::default() {
        return None;
    }
    if pads.iter().any(Option::is_some) {
        if !promises_one_line(context) {
            return None;
        }
        if pads.iter().flatten().any(|p| parse_inset_px(p).is_none()) {
            return None;
        }
    }
    let own_text = child.text.as_deref().filter(|t| !t.is_empty());
    match (own_text, child.children.as_slice()) {
        (Some(t), []) => Some(t),
        (None, [only]) if only.kind == GpuiElementKind::TextNode && only.children.is_empty() => {
            only.text.as_deref().filter(|t| !t.is_empty())
        }
        _ => None,
    }
}

/// One piece of a declared inline formatting context, in child order.
///
/// A context is NOT required to be fusable end to end. A row that interleaves
/// text with things that must keep their own element — an icon, a spacer, a
/// bracket carrying a margin — yields a mix of both variants, and
/// `build_plan_div` lays them out in exactly this order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineChunk {
    /// A maximal run of TWO OR MORE adjacent fusable spans, drawn as one
    /// text element so the run is rounded once instead of once per span.
    Fused(FusedRun),
    /// A child that keeps its own element, by index into `plan.children`.
    /// Carries no copy of the child: the plan is the single source of truth
    /// and the consumer renders `plan.children[i]` the ordinary way.
    Own(usize),
}

/// A maximal run of adjacent fusable spans, flattened into one string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FusedRun {
    /// Index into `plan.children` of the first span this run covers. Kept so
    /// a reader (and a test) can map a run back to the spans it came from;
    /// the spans themselves are still in the plan, untouched.
    pub start: usize,
    /// The concatenated text of the run's spans.
    pub text: String,
    /// The runs tiling `text`, one per span, in order.
    pub runs: Vec<InlineTextRun>,
}

/// Does the CONTAINER put something between its adjacent children that
/// fusing would delete?
///
/// This is the parent-side half of the guard in `fusable_child_text`, and
/// it has to be written by hand. The child-side probe tests a child's whole
/// `GpuiStyles` against the default, so a spacing field added to the struct
/// later is refused on a CHILD automatically. Nothing equivalent exists for
/// the container: most of its styling (a background, a padding, a height)
/// is legitimately kept by the fused row, because the container keeps its
/// own element. What is NOT kept is anything the layout places BETWEEN two
/// children, since fusing a run of n spans removes n-1 children and every
/// separation between them. A new field of that kind (a `column_gap`, a
/// `row_gap`) must be added here, or fusing will silently delete it.
///
/// - `gap` puts a gap between every pair of adjacent children: `N*(n-1)`
///   px of spacing per fused run. Any value is refused, `0` included,
///   rather than parsing it: refusing costs only the rounding fusion would
///   have removed, while a mis-parsed gap would delete real spacing.
/// - `justify-content: space-between` / `space-around` (and any other
///   `space-*` value) distribute the free space between children, so
///   fusing would redistribute it.
/// - `flex-direction` other than `row`: a column stacks the children, and
///   fusing would draw them on one line.
fn spaces_its_children(styles: &GpuiStyles) -> bool {
    styles.gap.is_some()
        || styles
            .justify
            .as_deref()
            .map_or(false, |j| j.starts_with("space"))
        || styles
            .flex_direction
            .as_deref()
            .map_or(false, |d| d != "row")
}

/// Is this a container that declared itself an inline formatting context?
fn declares_inline_runs(plan: &RenderNode) -> bool {
    plan.attributes
        .get(INLINE_TEXT_RUNS_ATTRIBUTE)
        .map_or(false, |v| v != "0" && v != "false")
}

/// `plan`'s children grouped into maximal runs of fusable spans, or `None`
/// when `plan` is not a declared inline formatting context or when nothing
/// in it would fuse.
///
/// WHY THIS IS SEGMENTED AND NOT ALL-OR-NOTHING. The first version of this
/// fused a container only when EVERY child was pure text, which made the
/// declaration a silent no-op on every row that interleaves text with
/// something else — and the rows that drift worst are exactly those: a
/// call-trace row carries a toggle icon, a spacer, a bracket with a margin
/// and a return arrow in another face between its text spans. Refusing to
/// fuse such a row leaves every one of its text spans rounded separately,
/// which is the whole defect. Grouping instead removes the rounding WITHIN
/// each run while leaving every non-text child exactly where it was, so the
/// spacing those children provide is preserved rather than concatenated
/// away.
///
/// A run of ONE span is deliberately NOT fused: one span is already one text
/// element, so fusing it would change nothing about layout while needlessly
/// replacing an element with a synthesised one.
///
/// THE PLAN IS NOT MODIFIED. Grouping happens here, in the plan's CONSUMER,
/// so every span — and every per-span attribute hung off it, such as the
/// call trace's `data-call-arg` — is still in the plan for hit-testing and
/// for tests to read. This is the property that lets a row fuse without
/// giving up per-span identity, and it is why fusing is not done by
/// rewriting the markup into fusable sub-groups.
pub fn inline_chunks(plan: &RenderNode) -> Option<Vec<InlineChunk>> {
    if !declares_inline_runs(plan) {
        return None;
    }
    // A container with text of its own is not purely a sequence of spans,
    // and `build_plan_div` would append that text AFTER the children.
    if plan.text.as_deref().map_or(false, |t| !t.is_empty()) {
        return None;
    }
    if plan.children.is_empty() {
        return None;
    }
    if spaces_its_children(&plan.styles) {
        return None;
    }

    let mut chunks: Vec<InlineChunk> = Vec::with_capacity(plan.children.len());
    let mut pending: Vec<(usize, &str)> = Vec::new();

    // Close the run accumulated so far: a real fusion at two spans or more,
    // otherwise the single span goes back to being its own element.
    //
    // A run that carries an inset is drawn by the inset element, which
    // shapes each segment as ONE line: GPUI's `shape_line` cannot take a
    // hard break. Such a run whose text holds one is therefore split back
    // into what the rule made of it before insets existed: every PADDED
    // span keeps its own element, and the plain spans between them still
    // fuse (they hold no inset, so a `StyledText` draws their break). A
    // break therefore never makes a row fuse less than it used to.
    fn flush<'a>(
        chunks: &mut Vec<InlineChunk>,
        plan: &RenderNode,
        pending: &mut Vec<(usize, &'a str)>,
    ) {
        if pending.len() < 2 {
            for (i, _) in pending.iter() {
                chunks.push(InlineChunk::Own(*i));
            }
            pending.clear();
            return;
        }
        let start = pending[0].0;
        let mut text = String::new();
        let mut runs = Vec::with_capacity(pending.len());
        for (i, piece) in pending.iter() {
            let styles = &plan.children[*i].styles;
            text.push_str(piece);
            runs.push(InlineTextRun {
                len: piece.len(),
                color: styles.text_color.clone(),
                font_weight: styles.font_weight.clone(),
                inset_left: styles.padding_left.clone(),
                inset_right: styles.padding_right.clone(),
            });
        }
        let one_line = !text.contains(['\n', '\r']);
        if one_line || !runs.iter().any(InlineTextRun::has_inset) {
            chunks.push(InlineChunk::Fused(FusedRun { start, text, runs }));
            pending.clear();
            return;
        }
        // Split at every span that declares a padding at all (a zero one
        // included, which the rule before insets refused too). `plain`
        // never holds a padded span, so the recursion cannot come back here.
        let mut plain: Vec<(usize, &'a str)> = Vec::new();
        for &(i, piece) in pending.iter() {
            let styles = &plan.children[i].styles;
            if styles.padding_left.is_some() || styles.padding_right.is_some() {
                flush(chunks, plan, &mut plain);
                chunks.push(InlineChunk::Own(i));
            } else {
                plain.push((i, piece));
            }
        }
        flush(chunks, plan, &mut plain);
        pending.clear();
    }

    for (i, child) in plan.children.iter().enumerate() {
        match fusable_child_text(plan, child) {
            Some(piece) => pending.push((i, piece)),
            None => {
                flush(&mut chunks, plan, &mut pending);
                chunks.push(InlineChunk::Own(i));
            }
        }
    }
    flush(&mut chunks, plan, &mut pending);

    // Nothing fused: tell the caller so, rather than handing back a plan
    // restated as `Own` entries it would render identically. This keeps the
    // "declared but nothing fusable" case on exactly the original code path.
    if !chunks.iter().any(|c| matches!(c, InlineChunk::Fused(_))) {
        return None;
    }
    Some(chunks)
}

/// The whole of `plan` fused into one string and its runs, or `None` when it
/// does not fuse end to end.
///
/// Retained because it states the ALL-FUSABLE case directly, which is both
/// the editor code row's shape and the simplest thing for a test to assert.
pub fn inline_text_runs(plan: &RenderNode) -> Option<(String, Vec<InlineTextRun>)> {
    match inline_chunks(plan)?.as_slice() {
        [InlineChunk::Fused(run)] if run.start == 0 && run.runs.len() == plan.children.len() => {
            Some((run.text.clone(), run.runs.clone()))
        }
        _ => None,
    }
}

/// Build a render plan from the shadow tree, starting at `root_id`.
///
/// This walks the tree recursively and produces a `RenderNode` hierarchy
/// that describes what GPUI elements to create. This intermediate
/// representation can be tested without GPUI dependencies.
pub fn build_render_plan(tree: &Tree, root_id: NodeId) -> Option<RenderNode> {
    let node = tree.get(root_id)?;
    let kind = classify_node(node);
    let styles = GpuiStyles::from_node(node);

    let tag = match &node.kind {
        NodeKind::Element(t) => t.clone(),
        NodeKind::Text(_) => "#text".to_string(),
    };

    let text = match &node.kind {
        NodeKind::Text(t) => Some(t.clone()),
        NodeKind::Element(_) => node.attributes.get("__text_content").cloned(),
    };

    let has_click_handler = node.event_listeners.contains_key("click");
    let has_input_handler = node.event_listeners.contains_key("input");
    let event_names: Vec<String> = node.event_listeners.keys().cloned().collect();
    let attributes: BTreeMap<String, String> = node
        .attributes
        .iter()
        .filter(|(k, _)| !k.starts_with("__"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let children: Vec<RenderNode> = node
        .children
        .iter()
        .filter_map(|&child_id| build_render_plan(tree, child_id))
        .collect();

    Some(RenderNode {
        node_id: root_id.0,
        kind,
        tag,
        styles,
        text,
        has_click_handler,
        has_input_handler,
        event_names,
        attributes,
        children,
    })
}

/// Count total render nodes in a plan (for testing).
pub fn count_render_nodes(plan: &RenderNode) -> usize {
    1 + plan
        .children
        .iter()
        .map(count_render_nodes)
        .sum::<usize>()
}

// ---------------------------------------------------------------------------
// GPUI rendering (only available with gpui-backend feature)
// ---------------------------------------------------------------------------

// RS-M14 Phase 2: the headless rendering path (`gpui-headless` feature) reuses
// `dispatch_shadow_event` and any helpers exposed by `gpui_render` so the
// off-screen capture pipeline produces the same output the windowed launch
// path produces. Widen the cfg gate accordingly.
#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
pub mod gpui_render {
    //! Actual GPUI element rendering from the shadow tree.
    //!
    //! This module contains the rendering functions that read the global shadow
    //! tree and produce GPUI elements via builder chains on each render cycle.

    use super::*;

    /// Dispatch an event to the shadow tree's event listeners.
    /// This is called from GPUI event handlers to bridge back to the
    /// Nim-side callbacks.
    ///
    /// The lock is released before calling callbacks to avoid deadlocks
    /// (callbacks may mutate the tree via FFI).
    ///
    /// Supports both legacy function pointer listeners (callback_id == 0)
    /// and dispatcher-based listeners (callback_id > 0).
    pub fn dispatch_shadow_event(node_id: NodeId, event_name: &str) {
        // PLAT-38: this WAS a byte-identical copy of `gpui_dispatch_event`'s
        // body. Two copies of one predicate is §30 — and this copy is the one
        // a real compositor event runs, while almost every case in the
        // campaign drives the other, so the suites were grading a different
        // function from the one the window uses. One routine now; the payload
        // is `None` because a mouse-up carries no key.
        crate::input::deliver(node_id, event_name, None);
    }
}

// ---------------------------------------------------------------------------
// Tests (no GPUI dependency required)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{EventListener, Node, NodeId, Tree};

    /// A syntax-highlighted code row as `leaves.nim` builds it: a container
    /// holding one span per coloured piece, each span holding one text node.
    /// `extra` runs last on the FIRST span, so a test can make exactly one
    /// child unfusable and watch the whole row decline.
    fn highlighted_row(
        declare: bool,
        pieces: &[(&str, &str)],
        extra: impl FnOnce(&mut Node),
    ) -> RenderNode {
        let mut tree = Tree::new();
        let mut code = Node::new_element("span");
        code.styles.insert("display".into(), "flex".into());
        if declare {
            code.attributes
                .insert(INLINE_TEXT_RUNS_ATTRIBUTE.into(), "1".into());
        }
        let code_id = tree.insert(code);
        // `Option::take` so the FnOnce can be consumed inside the loop.
        let mut extra = Some(extra);
        for (text, color) in pieces {
            let mut span = Node::new_element("span");
            span.styles.insert("color".into(), (*color).into());
            span.styles.insert("flex-shrink".into(), "0".into());
            if let Some(f) = extra.take() {
                f(&mut span);
            }
            let span_id = tree.insert(span);
            let text_id = tree.insert(Node::new_text(text));
            tree.append_child(span_id, text_id);
            tree.append_child(code_id, span_id);
        }
        build_render_plan(&tree, code_id).expect("plan")
    }

    /// The motivating case: a declared row of coloured spans fuses into one
    /// string whose runs cover it exactly, in order, with each span's colour.
    #[test]
    fn inline_runs_fuse_a_declared_highlighted_row() {
        let plan = highlighted_row(
            true,
            &[("let", "#569cd6"), (" x = ", "#d4d4d4"), ("(", "#ffd700")],
            |_| {},
        );
        let (text, runs) = inline_text_runs(&plan).expect("declared row must fuse");
        assert_eq!(text, "let x = (");
        assert_eq!(
            runs,
            vec![
                InlineTextRun {
                    len: 3,
                    color: Some("#569cd6".into()),
                    font_weight: None,
                    inset_left: None,
                    inset_right: None
                },
                InlineTextRun {
                    len: 5,
                    color: Some("#d4d4d4".into()),
                    font_weight: None,
                    inset_left: None,
                    inset_right: None
                },
                InlineTextRun {
                    len: 1,
                    color: Some("#ffd700".into()),
                    font_weight: None,
                    inset_left: None,
                    inset_right: None
                },
            ]
        );
        // The runs must tile the fused string with no gap and no overlap --
        // that is precisely the contract `with_highlights` is handed.
        assert_eq!(runs.iter().map(|r| r.len).sum::<usize>(), text.len());
    }

    /// THE NEGATIVE CONTROL FOR EVERY TEST ABOVE AND BELOW. The same row
    /// without the attribute must NOT fuse, which is what shows the fusing
    /// is driven by the declaration rather than happening to everything.
    #[test]
    fn inline_runs_decline_a_row_that_did_not_declare_itself() {
        let plan = highlighted_row(false, &[("let", "#569cd6"), (" x", "#d4d4d4")], |_| {});
        assert_eq!(inline_text_runs(&plan), None);
    }

    /// TWO SPANS THAT DIFFER ONLY IN WEIGHT FUSE, and the weight rides along
    /// on the run. This is the property that lets a caller draw a label and
    /// a bold value as one piece of text, and it is the reason `font-weight`
    /// was added to the fusable set.
    #[test]
    fn inline_runs_fuse_spans_that_differ_only_in_weight() {
        let mut tree = Tree::new();
        let mut host = Node::new_element("span");
        host.attributes
            .insert(INLINE_TEXT_RUNS_ATTRIBUTE.into(), "1".into());
        let host_id = tree.insert(host);
        for (text, weight) in [("name", None), ("value", Some("600"))] {
            let mut span = Node::new_element("span");
            span.styles.insert("color".into(), "#d4d4d4".into());
            span.styles.insert("flex-shrink".into(), "0".into());
            if let Some(w) = weight {
                span.styles.insert("font-weight".into(), w.into());
            }
            let span_id = tree.insert(span);
            let text_id = tree.insert(Node::new_text(text));
            tree.append_child(span_id, text_id);
            tree.append_child(host_id, span_id);
        }
        let plan = build_render_plan(&tree, host_id).expect("plan");
        let (text, runs) = inline_text_runs(&plan).expect("must fuse");
        assert_eq!(text, "namevalue");
        assert_eq!(runs[0].font_weight, None, "inherits when unset");
        assert_eq!(runs[1].font_weight, Some("600".into()));
        assert_eq!(runs.iter().map(|r| r.len).sum::<usize>(), text.len());
    }

    /// THE GUARD ON THE PADDING GAP. A span with leading padding must NOT
    /// fuse in a context that has not promised one line (no `white-space:
    /// nowrap` / `pre` on the container, as here). A `StyledText` highlight
    /// cannot carry padding, and the inset element that can draws only one
    /// line and never truncates -- so outside a one-line context there is
    /// nothing that reproduces the gap exactly, and fusing would either
    /// delete it (measured on aarch64-darwin 2026-10-21 against codetracer
    /// 3fce565eb: an editor chip's name and value are ~8.5 pt apart and BOTH
    /// front-ends draw that gap) or draw one line where the spans may wrap.
    /// The positive half -- the same padding fusing in a nowrap context with
    /// the gap carried as an inset -- is
    /// `inline_chunks_fuse_a_nowrap_chips_name_and_padded_value`.
    #[test]
    fn inline_runs_decline_a_padded_span_outside_a_one_line_context() {
        let plan = highlighted_row(true, &[("name", "#9cdcfe"), ("value", "#d4d4d4")],
                                   |span| {
            span.styles.insert("padding-left".into(), "7px".into());
        });
        assert_eq!(plan.styles.white_space, None, "the context promised nothing");
        assert_eq!(inline_text_runs(&plan), None);
    }

    /// `font-style` is NOT in the fusable set although a `HighlightStyle`
    /// carries it, because nothing in this shim applies `font-style` to an
    /// unfused span -- there is no `italic` call anywhere. Fusing it would
    /// make the same markup render italic only when it happened to fuse,
    /// which would turn fusing from a layout change into a drawing change.
    /// When the unfused path learns to draw it, this test is the one to
    /// revisit, together with the comment in `fusable_child_text`.
    #[test]
    fn inline_runs_decline_a_span_with_a_font_style() {
        let plan = highlighted_row(true, &[("let", "#569cd6"), (" x", "#d4d4d4")],
                                   |span| {
            span.styles.insert("font-style".into(), "italic".into());
        });
        assert_eq!(inline_text_runs(&plan), None);
    }

    /// A span that can be clicked is not just text: fusing it away would
    /// delete the hit target, so the whole row keeps its own elements.
    #[test]
    fn inline_runs_decline_a_row_whose_span_has_a_handler() {
        extern "C" fn noop(_p: *const crate::input::GpuiEventPayload) {}
        let plan = highlighted_row(true, &[("let", "#569cd6"), (" x", "#d4d4d4")], |span| {
            span.event_listeners
                .entry("click".into())
                .or_default()
                .push(EventListener {
                    callback: noop,
                    callback_id: 0,
                });
        });
        assert_eq!(inline_text_runs(&plan), None);
    }

    /// A span with box styling (here a background) is painted in its own
    /// right. The check is by difference from the default style, so this
    /// also guards every property nobody has thought of yet.
    #[test]
    fn inline_runs_decline_a_row_whose_span_has_box_styling() {
        let plan = highlighted_row(true, &[("let", "#569cd6"), (" x", "#d4d4d4")], |span| {
            span.styles.insert("background".into(), "#333".into());
        });
        assert_eq!(inline_text_runs(&plan), None);
    }

    /// A span declaring a window region acts on the window when pressed.
    #[test]
    fn inline_runs_decline_a_row_whose_span_is_a_window_region() {
        let plan = highlighted_row(true, &[("let", "#569cd6"), (" x", "#d4d4d4")], |span| {
            span.attributes
                .insert(crate::regions::REGION_ATTRIBUTE.into(), "drag".into());
        });
        assert_eq!(inline_text_runs(&plan), None);
    }

    /// A span with no colour of its own keeps the inherited one: the run is
    /// still emitted, so the runs go on tiling the string.
    #[test]
    fn inline_runs_keep_an_uncoloured_span_as_an_inherited_run() {
        let mut tree = Tree::new();
        let mut code = Node::new_element("span");
        code.attributes
            .insert(INLINE_TEXT_RUNS_ATTRIBUTE.into(), "1".into());
        let code_id = tree.insert(code);
        for (text, color) in [("ab", Some("#111111")), ("cd", None)] {
            let mut span = Node::new_element("span");
            if let Some(c) = color {
                span.styles.insert("color".into(), c.into());
            }
            let span_id = tree.insert(span);
            let text_id = tree.insert(Node::new_text(text));
            tree.append_child(span_id, text_id);
            tree.append_child(code_id, span_id);
        }
        let plan = build_render_plan(&tree, code_id).expect("plan");
        let (text, runs) = inline_text_runs(&plan).expect("must fuse");
        assert_eq!(text, "abcd");
        assert_eq!(runs[1].color, None);
        assert_eq!(runs.iter().map(|r| r.len).sum::<usize>(), text.len());
    }

    /// Run lengths are BYTE lengths, because `with_highlights` takes byte
    /// ranges. A row of multi-byte characters is where a character count
    /// would silently produce ranges that land mid-character.
    #[test]
    fn inline_runs_measure_multibyte_spans_in_bytes() {
        let plan = highlighted_row(true, &[("λx", "#569cd6"), ("→y", "#d4d4d4")], |_| {});
        let (text, runs) = inline_text_runs(&plan).expect("must fuse");
        assert_eq!(runs[0].len, "λx".len());
        assert_eq!(runs[1].len, "→y".len());
        assert_ne!(runs[0].len, "λx".chars().count(), "byte length, not chars");
        assert_eq!(runs.iter().map(|r| r.len).sum::<usize>(), text.len());
        // Every boundary the ranges imply must be a character boundary.
        let mut at = 0usize;
        for run in &runs {
            assert!(text.is_char_boundary(at));
            at += run.len;
        }
        assert!(text.is_char_boundary(at));
    }

    // ---- SEGMENTED FUSION (`inline_chunks`) --------------------------
    //
    // The all-or-nothing tests above still hold and are the back-compat
    // floor: a row whose children are ALL fusable must still collapse to a
    // single run covering every child, which is what `inline_text_runs`
    // asserts for them. The tests below cover what segmenting ADDS — a row
    // that interleaves text with children that must keep their own element.

    /// Build a row that interleaves fusable spans with children that are
    /// not fusable, as a call-trace row does. `spec` is read left to right:
    /// `Some(text)` is a plain coloured span, `None` is a child that cannot
    /// fuse (here a sized box, standing in for the toggle icon or spacer).
    fn mixed_row(spec: &[Option<&str>]) -> RenderNode {
        let mut tree = Tree::new();
        let mut host = Node::new_element("div");
        host.attributes
            .insert(INLINE_TEXT_RUNS_ATTRIBUTE.into(), "1".into());
        let host_id = tree.insert(host);
        for item in spec {
            match item {
                Some(t) => {
                    let mut span = Node::new_element("span");
                    span.styles.insert("color".into(), "#d4d4d4".into());
                    span.styles.insert("flex-shrink".into(), "0".into());
                    let span_id = tree.insert(span);
                    let text_id = tree.insert(Node::new_text(t));
                    tree.append_child(span_id, text_id);
                    tree.append_child(host_id, span_id);
                }
                None => {
                    let mut box_ = Node::new_element("div");
                    // A width is box styling, so this child cannot fuse --
                    // the same reason the call trace's 8 px spacer cannot.
                    box_.styles.insert("width".into(), "8px".into());
                    let box_id = tree.insert(box_);
                    tree.append_child(host_id, box_id);
                }
            }
        }
        build_render_plan(&tree, host_id).expect("plan")
    }

    /// Collect the shape of the chunking as a compact description, so a
    /// test asserts the WHOLE layout order rather than one chunk of it.
    fn shape(plan: &RenderNode) -> Vec<String> {
        inline_chunks(plan)
            .expect("must chunk")
            .iter()
            .map(|c| match c {
                InlineChunk::Fused(r) => format!("fused@{}:{}", r.start, r.text),
                InlineChunk::Own(i) => format!("own@{i}"),
            })
            .collect()
    }

    /// THE MOTIVATING CASE FOR SEGMENTING. A row that interleaves text with
    /// a non-fusable child fuses each maximal run of text and leaves the
    /// other child exactly where it was. Before segmenting this row fused
    /// NOTHING -- the declaration was a silent no-op -- and every one of its
    /// four text spans was rounded separately.
    #[test]
    fn inline_chunks_fuse_each_run_around_a_non_fusable_child() {
        let plan = mixed_row(&[Some("foo"), Some("()"), None, Some(" => "), Some("42")]);
        assert_eq!(
            shape(&plan),
            vec!["fused@0:foo()", "own@2", "fused@3: => 42"]
        );
    }

    /// THE NEGATIVE CONTROL FOR SEGMENTING, the counterpart of
    /// `inline_runs_decline_a_row_that_did_not_declare_itself`: the SAME
    /// interleaved row without the attribute must not chunk at all, so the
    /// grouping is driven by the declaration and not by the shape.
    #[test]
    fn inline_chunks_decline_an_interleaved_row_that_did_not_declare_itself() {
        let mut plan = mixed_row(&[Some("foo"), Some("()"), None, Some("42")]);
        plan.attributes.remove(INLINE_TEXT_RUNS_ATTRIBUTE);
        assert_eq!(inline_chunks(&plan), None);
    }

    /// A RUN OF ONE IS NOT FUSED. One span is already one text element, so
    /// fusing it would change no layout while replacing a real element with
    /// a synthesised one. Here every text span is isolated by a box, so
    /// there is no run of two anywhere and the whole row declines.
    #[test]
    fn inline_chunks_decline_a_row_whose_text_spans_are_all_isolated() {
        let plan = mixed_row(&[Some("a"), None, Some("b"), None, Some("c")]);
        assert_eq!(inline_chunks(&plan), None);
    }

    /// A row with no fusable child at all declines, rather than restating
    /// every child as an `Own` entry the consumer would render identically.
    #[test]
    fn inline_chunks_decline_a_row_with_no_fusable_child() {
        let plan = mixed_row(&[None, None]);
        assert_eq!(inline_chunks(&plan), None);
    }

    /// BACK-COMPAT, STATED AS CHUNKS. The editor code row -- every child
    /// fusable -- must still be ONE run covering every child from index 0.
    /// This is the property `inline_text_runs` now reads off `inline_chunks`,
    /// and the editor pane's measured parity depends on it not changing.
    #[test]
    fn inline_chunks_fuse_an_all_fusable_row_as_one_run() {
        let plan = highlighted_row(
            true,
            &[("let", "#569cd6"), (" x = ", "#d4d4d4"), ("(", "#ffd700")],
            |_| {},
        );
        assert_eq!(shape(&plan), vec!["fused@0:let x = ("]);
        // And the all-or-nothing view of the same plan still answers.
        let (text, runs) = inline_text_runs(&plan).expect("all-fusable row");
        assert_eq!(text, "let x = (");
        assert_eq!(runs.len(), 3);
    }

    /// `inline_text_runs` is the ALL-FUSABLE view and must NOT report a
    /// partially fused row as if the whole of it had fused -- otherwise a
    /// caller reading it would believe a row collapsed to one element when
    /// three remain. The row below does chunk; it just does not chunk whole.
    #[test]
    fn inline_text_runs_decline_a_row_that_only_partly_fuses() {
        let plan = mixed_row(&[Some("foo"), Some("()"), None, Some(" => "), Some("42")]);
        assert!(inline_chunks(&plan).is_some(), "it does chunk");
        assert_eq!(inline_text_runs(&plan), None, "but not as a whole");
    }

    /// Each fused run's byte lengths tile ITS OWN text exactly -- the
    /// contract `with_highlights` is handed, now asserted per run rather
    /// than once for the row, and over multi-byte spans.
    #[test]
    fn inline_chunks_tile_each_run_in_bytes() {
        let plan = mixed_row(&[
            Some("\u{3bb}x"),
            Some("\u{2192}y"),
            None,
            Some("ab"),
            Some("cd"),
        ]);
        let chunks = inline_chunks(&plan).expect("must chunk");
        let mut fused = 0;
        for chunk in &chunks {
            if let InlineChunk::Fused(run) = chunk {
                fused += 1;
                assert_eq!(
                    run.runs.iter().map(|r| r.len).sum::<usize>(),
                    run.text.len()
                );
                let mut at = 0usize;
                for r in &run.runs {
                    assert!(run.text.is_char_boundary(at));
                    at += r.len;
                }
                assert!(run.text.is_char_boundary(at));
            }
        }
        assert_eq!(fused, 2, "both runs fused");
    }

    /// A non-fusable child is referenced BY INDEX into the plan, and the
    /// plan still holds every span with its own attributes. This is what
    /// lets the call trace fuse a row while keeping `data-call-arg` on each
    /// argument for hit-testing -- the grouping is a rendering decision, not
    /// a rewrite of the markup.
    #[test]
    fn inline_chunks_leave_the_plan_and_its_per_span_attributes_intact() {
        let mut tree = Tree::new();
        let mut host = Node::new_element("div");
        host.attributes
            .insert(INLINE_TEXT_RUNS_ATTRIBUTE.into(), "1".into());
        let host_id = tree.insert(host);
        for (text, arg) in [("a", Some("0")), ("b", Some("1"))] {
            let mut span = Node::new_element("span");
            span.styles.insert("color".into(), "#d4d4d4".into());
            if let Some(a) = arg {
                span.attributes.insert("data-call-arg".into(), a.into());
            }
            let span_id = tree.insert(span);
            let text_id = tree.insert(Node::new_text(text));
            tree.append_child(span_id, text_id);
            tree.append_child(host_id, span_id);
        }
        let plan = build_render_plan(&tree, host_id).expect("plan");
        assert_eq!(shape(&plan), vec!["fused@0:ab"]);
        // The spans the run was built from are still there, attributes and
        // all: fusing consumed nothing.
        assert_eq!(plan.children.len(), 2);
        assert_eq!(
            plan.children[0].attributes.get("data-call-arg"),
            Some(&"0".to_string())
        );
        assert_eq!(
            plan.children[1].attributes.get("data-call-arg"),
            Some(&"1".to_string())
        );
    }

    /// PLAT-50's ACTUAL GUARANTEE, stated per span. A call-trace argument
    /// is clickable: `window_clicks.wireWindowClicks` walks the shadow tree
    /// and hangs `mousedown` / `contextmenu` / `auxdown` on every span
    /// carrying `data-call-arg`, so the span's right-click menu works. A
    /// fused run is ONE synthesised `StyledText` and the spans it covers get
    /// no element of their own -- so if a wired span could fuse, its
    /// listeners would have nothing to attach to and the menu would die
    /// silently.
    ///
    /// It cannot, and this is what makes PLAT-50 safe: `fusable_child_text`
    /// rejects any child with a listener BEFORE it looks at anything else.
    /// The row below is fusable end to end except for the wired span in the
    /// middle, which must come back as its own element with its neighbours
    /// fused around it -- not swallowed into either run.
    ///
    /// The sibling test above shows the plan keeps `data-call-arg`; this one
    /// shows the span keeps its ELEMENT, which is the half that actually
    /// delivers the click.
    #[test]
    fn inline_chunks_keep_a_wired_span_as_its_own_element() {
        extern "C" fn noop(_p: *const crate::input::GpuiEventPayload) {}
        let mut tree = Tree::new();
        let mut host = Node::new_element("div");
        host.attributes
            .insert(INLINE_TEXT_RUNS_ATTRIBUTE.into(), "1".into());
        let host_id = tree.insert(host);
        for (text, wired) in [
            ("f", false),
            ("(", false),
            ("arg", true),
            (")", false),
            (" => 1", false),
        ] {
            let mut span = Node::new_element("span");
            span.styles.insert("color".into(), "#d4d4d4".into());
            if wired {
                span.attributes.insert("data-call-arg".into(), "0".into());
                for name in ["mousedown", "contextmenu", "auxdown"] {
                    span.event_listeners
                        .entry(name.into())
                        .or_default()
                        .push(EventListener {
                            callback: noop,
                            callback_id: 0,
                        });
                }
            }
            let span_id = tree.insert(span);
            let text_id = tree.insert(Node::new_text(text));
            tree.append_child(span_id, text_id);
            tree.append_child(host_id, span_id);
        }
        let plan = build_render_plan(&tree, host_id).expect("plan");
        // The wired argument keeps its own element; the plain text each side
        // of it still fuses, so the parity win is kept where it is free.
        assert_eq!(shape(&plan), vec!["fused@0:f(", "own@2", "fused@3:) => 1"]);
        // POSITIVE CONTROL: the same row WITHOUT the listeners fuses whole,
        // so the `own@2` above is caused by the listeners and not by the
        // row's shape or by `data-call-arg` itself.
        let mut unwired = plan.clone();
        unwired.children[2].event_names.clear();
        unwired.children[2].has_click_handler = false;
        assert_eq!(shape(&unwired), vec!["fused@0:f(arg) => 1"]);
    }

    /// EVERY DECLINE GUARD ABOVE STILL DECLINES PER SPAN, not just per row.
    /// In a context that has NOT promised one line, a span with leading
    /// padding inside an otherwise fusable row keeps its own element -- the
    /// guard's point is that nothing could reproduce the gap there, and
    /// segmenting must not quietly reintroduce the deletion. The POSITIVE
    /// CONTROL at the end shows the same row in a nowrap context fusing
    /// whole with the 5 px carried as an inset, so the `own@1` is caused by
    /// the missing one-line promise and nothing else.
    #[test]
    fn inline_chunks_keep_a_padded_span_as_its_own_element_outside_a_one_line_context() {
        let plan = padded_row(None);
        // "a" is a run of one, so it keeps its element too; the padded "("
        // is its own; "bc" fuses. The 5 px is preserved either way.
        assert_eq!(shape(&plan), vec!["own@0", "own@1", "fused@2:bc"]);

        let plan = padded_row(Some("nowrap"));
        assert_eq!(shape(&plan), vec!["fused@0:a(bc"]);
        let Some(InlineChunk::Fused(run)) = inline_chunks(&plan).and_then(|c| c.into_iter().next())
        else {
            panic!("must fuse");
        };
        assert_eq!(run.runs[1].inset_left.as_deref(), Some("5px"));
        assert!(run.runs[1].has_inset());
        assert!(!run.runs[0].has_inset() && !run.runs[2].has_inset());
    }

    /// The row the padded-span tests read (`inline_chunks_keep_a_padded_span_
    /// as_its_own_element_...`, `inline_chunks_split_a_broken_inset_run_...`):
    /// "a", a 5 px-padded "(", "b", "c", in a flex context that declares
    /// `white_space` when it is `Some`.
    fn padded_row(white_space: Option<&str>) -> RenderNode {
        let mut tree = Tree::new();
        let mut host = Node::new_element("div");
        // A flex row, as every real caller is, so the fixture stays valid
        // under a container rule that refuses a block container.
        host.styles.insert("display".into(), "flex".into());
        host.attributes
            .insert(INLINE_TEXT_RUNS_ATTRIBUTE.into(), "1".into());
        if let Some(ws) = white_space {
            host.styles.insert("white-space".into(), ws.into());
        }
        let host_id = tree.insert(host);
        for (text, pad) in [("a", false), ("(", true), ("b", false), ("c", false)] {
            let mut span = Node::new_element("span");
            span.styles.insert("color".into(), "#d4d4d4".into());
            if pad {
                span.styles.insert("padding-left".into(), "5px".into());
            }
            let span_id = tree.insert(span);
            let text_id = tree.insert(Node::new_text(text));
            tree.append_child(span_id, text_id);
            tree.append_child(host_id, span_id);
        }
        build_render_plan(&tree, host_id).expect("plan")
    }

    // ---- HORIZONTAL INSETS (`padding-left` / `padding-right`) ---------
    //
    // The editor's inline-value chip, as codetracer's `leaves.nim` builds
    // it: a box holding an optional "view more" box, the name, and the
    // value spaced 7 px off the name by its own `padding-left`. These tests
    // pin the rule under which that padding now fuses (the context promised
    // one line) and every case in which it still must not.

    /// A chip-shaped declared context. `white_space` / `text_overflow` are
    /// the CONTAINER's declarations; `more` puts the "view more" box (a
    /// sized box holding an `<img>`) before the name; `value_style` is
    /// applied last to the value span so a test can vary its padding.
    fn chip(
        white_space: Option<&str>,
        text_overflow: Option<&str>,
        more: bool,
        value_style: &[(&str, &str)],
    ) -> RenderNode {
        let mut tree = Tree::new();
        let mut host = Node::new_element("div");
        host.styles.insert("display".into(), "flex".into());
        host.styles.insert("padding-left".into(), "7px".into());
        host.styles.insert("background-color".into(), "#2a2a2a".into());
        if let Some(ws) = white_space {
            host.styles.insert("white-space".into(), ws.into());
        }
        if let Some(to) = text_overflow {
            host.styles.insert("text-overflow".into(), to.into());
        }
        host.attributes
            .insert(INLINE_TEXT_RUNS_ATTRIBUTE.into(), "1".into());
        let host_id = tree.insert(host);
        if more {
            let mut more_box = Node::new_element("div");
            more_box.styles.insert("width".into(), "30px".into());
            more_box
                .attributes
                .insert("data-ct-inline-more".into(), "x".into());
            let more_id = tree.insert(more_box);
            let mut eye = Node::new_element("img");
            eye.attributes.insert("src".into(), "/eye.svg".into());
            let eye_id = tree.insert(eye);
            tree.append_child(more_id, eye_id);
            tree.append_child(host_id, more_id);
        }
        let mut name = Node::new_element("div");
        name.styles.insert("color".into(), "#9cdcfe".into());
        name.styles.insert("flex-shrink".into(), "0".into());
        let name_id = tree.insert(name);
        let name_text = tree.insert(Node::new_text("x"));
        tree.append_child(name_id, name_text);
        tree.append_child(host_id, name_id);
        let mut value = Node::new_element("div");
        value.styles.insert("color".into(), "#d4d4d4".into());
        value.styles.insert("font-weight".into(), "600".into());
        value.styles.insert("flex-shrink".into(), "0".into());
        for (k, v) in value_style {
            value.styles.insert((*k).into(), (*v).into());
        }
        let value_id = tree.insert(value);
        let value_text = tree.insert(Node::new_text("1"));
        tree.append_child(value_id, value_text);
        tree.append_child(host_id, value_id);
        build_render_plan(&tree, host_id).expect("plan")
    }

    /// THE CHIP THIS CAPABILITY EXISTS FOR. A nowrap chip's name and its
    /// 7 px-padded value fuse into ONE run, so the chip's text is rounded
    /// once instead of once per span.
    #[test]
    fn inline_chunks_fuse_a_nowrap_chips_name_and_padded_value() {
        let plan = chip(Some("nowrap"), None, false, &[("padding-left", "7px")]);
        assert_eq!(shape(&plan), vec!["fused@0:x1"]);
        let (text, runs) = inline_text_runs(&plan).expect("the chip fuses whole");
        assert_eq!(text, "x1");
        assert_eq!(
            runs,
            vec![
                InlineTextRun {
                    len: 1,
                    color: Some("#9cdcfe".into()),
                    font_weight: None,
                    inset_left: None,
                    inset_right: None,
                },
                InlineTextRun {
                    len: 1,
                    color: Some("#d4d4d4".into()),
                    font_weight: Some("600".into()),
                    // THE GAP IS KEPT: carried as the value's leading inset.
                    inset_left: Some("7px".into()),
                    inset_right: None,
                },
            ]
        );
        assert_eq!(runs[1].inset_left_px(), 7.0);
        assert!(runs[1].has_inset() && !runs[0].has_inset());
    }

    /// With the "view more" box first, the box keeps its own element (it is
    /// a sized box holding an image) and the name and value fuse after it.
    #[test]
    fn inline_chunks_fuse_a_chips_text_after_its_view_more_box() {
        let plan = chip(Some("nowrap"), None, true, &[("padding-left", "7px")]);
        assert_eq!(shape(&plan), vec!["own@0", "fused@1:x1"]);
        let chunks = inline_chunks(&plan).expect("must chunk");
        let InlineChunk::Fused(run) = &chunks[1] else {
            panic!("the name and value fuse");
        };
        assert_eq!(run.runs[1].inset_left.as_deref(), Some("7px"));
    }

    /// A TRAILING inset rides on its run as `inset_right`.
    #[test]
    fn inline_chunks_carry_a_trailing_inset() {
        let plan = chip(Some("nowrap"), None, false, &[("padding-right", "3")]);
        let (_, runs) = inline_text_runs(&plan).expect("must fuse");
        assert_eq!(runs[1].inset_right.as_deref(), Some("3"));
        assert_eq!(runs[1].inset_right_px(), 3.0);
        assert_eq!(runs[1].inset_left, None);
    }

    /// A run that would carry an inset but holds a HARD LINE BREAK is not
    /// fused: the inset element shapes each segment as one line. Its spans
    /// keep their own elements, as before insets existed. The control: the
    /// same break WITHOUT padding still fuses as it always did (a
    /// `StyledText` draws the break itself).
    #[test]
    fn inline_chunks_keep_an_inset_run_that_holds_a_line_break() {
        let mut plan = chip(Some("nowrap"), None, false, &[("padding-left", "7px")]);
        plan.children[0].children[0].text = Some("a\nb".into());
        assert_eq!(inline_chunks(&plan), None);
        let mut control = chip(Some("nowrap"), None, false, &[]);
        control.children[0].children[0].text = Some("a\nb".into());
        assert_eq!(shape(&control), vec!["fused@0:a\nb1"]);
    }

    /// A broken inset run is split back into what the rule made of it
    /// before insets existed, not dropped whole: the padded span keeps its
    /// own element and the plain spans after it still fuse, break and all.
    /// The control is the same row without the break, which fuses whole
    /// with the padding as an inset.
    #[test]
    fn inline_chunks_split_a_broken_inset_run_at_its_padded_spans() {
        let plan = padded_row(Some("pre"));
        assert_eq!(shape(&plan), vec!["fused@0:a(bc"]);
        let mut plan = padded_row(Some("pre"));
        plan.children[2].children[0].text = Some("b\nx".into());
        assert_eq!(shape(&plan), vec!["own@0", "own@1", "fused@2:b\nxc"]);
        let Some(chunks) = inline_chunks(&plan) else {
            panic!("the plain spans still fuse");
        };
        let InlineChunk::Fused(run) = &chunks[2] else {
            panic!("chunk 2 is the fused tail");
        };
        assert!(!run.runs.iter().any(InlineTextRun::has_inset));
    }

    /// A zero padding is carried but is no inset at all, so such a run is
    /// drawn exactly as an unpadded one.
    #[test]
    fn inline_chunks_treat_a_zero_padding_as_no_inset() {
        let plan = chip(Some("nowrap"), None, false, &[("padding-left", "0px")]);
        let (_, runs) = inline_text_runs(&plan).expect("must fuse");
        assert!(!runs.iter().any(InlineTextRun::has_inset));
    }

    /// `white-space: pre` promises one line as well as `nowrap` does.
    #[test]
    fn inline_chunks_fuse_a_padded_span_in_a_pre_context() {
        let plan = chip(Some("pre"), None, false, &[("padding-left", "7px")]);
        assert_eq!(shape(&plan), vec!["fused@0:x1"]);
    }

    /// The SAME chip without the one-line promise must keep the padded
    /// value as its own element: a context that may wrap or truncate is
    /// one the inset element cannot draw.
    #[test]
    fn inline_chunks_keep_a_padded_span_in_a_context_that_may_wrap() {
        let plan = chip(None, None, false, &[("padding-left", "7px")]);
        assert_eq!(inline_chunks(&plan), None);
        let plan = chip(Some("normal"), None, false, &[("padding-left", "7px")]);
        assert_eq!(inline_chunks(&plan), None);
    }

    /// A nowrap context that ALSO declares `text-overflow` may truncate,
    /// which the inset element never does, so the padded span declines.
    #[test]
    fn inline_chunks_keep_a_padded_span_in_a_context_that_may_truncate() {
        let plan = chip(Some("nowrap"), Some("ellipsis"), false, &[("padding-left", "7px")]);
        assert_eq!(inline_chunks(&plan), None);
    }

    /// A padding that is not a non-negative px length cannot be reproduced
    /// as an exact inset, so the span declines rather than guess.
    #[test]
    fn inline_chunks_keep_a_span_whose_padding_does_not_parse() {
        for bad in ["1em", "-3px", "auto", "", "NaNpx", "infpx", "50%", "7 px"] {
            let plan = chip(Some("nowrap"), None, false, &[("padding-left", bad)]);
            assert_eq!(inline_chunks(&plan), None, "padding-left {bad:?}");
            let plan = chip(Some("nowrap"), None, false, &[("padding-right", bad)]);
            assert_eq!(inline_chunks(&plan), None, "padding-right {bad:?}");
        }
    }

    /// Only HORIZONTAL padding is an inset: `padding-top` changes the
    /// span's box height, which a run on one line cannot state.
    #[test]
    fn inline_chunks_keep_a_span_with_vertical_padding() {
        for side in ["padding-top", "padding-bottom"] {
            let plan = chip(Some("nowrap"), None, false, &[(side, "2px")]);
            assert_eq!(inline_chunks(&plan), None, "{side}");
        }
    }

    /// The `padding` SHORTHAND pads all four sides, so it still declines
    /// even when it is a plain px length in a nowrap context.
    #[test]
    fn inline_chunks_keep_a_span_with_the_padding_shorthand() {
        let plan = chip(Some("nowrap"), None, false, &[("padding", "7px")]);
        assert_eq!(inline_chunks(&plan), None);
    }

    /// Test-by-difference still holds for everything the inset does not
    /// carry: a margin, or the SPAN's own `white-space`, still declines.
    #[test]
    fn inline_chunks_keep_a_padded_span_with_any_other_style() {
        for (k, v) in [("margin-left", "7px"), ("white-space", "nowrap"), ("background", "#333")] {
            let plan = chip(Some("nowrap"), None, false, &[("padding-left", "7px"), (k, v)]);
            assert_eq!(inline_chunks(&plan), None, "{k}: {v}");
        }
    }

    /// THE CONTAINER-SIDE COUNTERPART OF THE PADDED-SPAN GUARD. A flex
    /// container's `gap` puts space BETWEEN adjacent children, so fusing a
    /// run of n spans removes n-1 children and n-1 gaps with them -- the
    /// same spacing the padded-span guard protects, expressed on the parent
    /// instead of the child. The child-side probe cannot see it, so the
    /// container is refused outright. `justify-content: space-between` and
    /// `space-around` distribute free space between children the same way,
    /// and `flex-direction: column` puts a line break between them; each is
    /// refused for the same reason.
    #[test]
    fn inline_chunks_decline_a_container_that_spaces_its_children() {
        // POSITIVE CONTROL: the bare row fuses whole, so each decline below
        // is caused by the container property and not by the row's shape.
        let plan = mixed_row(&[Some("a"), Some("b"), Some("c")]);
        assert_eq!(shape(&plan), vec!["fused@0:abc"]);
        for (key, value) in [
            ("gap", "8px"),
            ("gap", "0"),
            ("justify-content", "space-between"),
            ("justify-content", "space-around"),
            ("justify-content", "space-evenly"),
            ("flex-direction", "column"),
        ] {
            let mut plan = mixed_row(&[Some("a"), Some("b"), Some("c")]);
            plan.styles.apply(key, value);
            assert_eq!(inline_chunks(&plan), None, "container {key}: {value}");
            assert_eq!(inline_text_runs(&plan), None, "container {key}: {value}");
        }
        // Properties that only ALIGN the children, or state the default
        // direction, add nothing between them and must not decline.
        for (key, value) in [
            ("justify-content", "start"),
            ("align-items", "center"),
            ("flex-direction", "row"),
        ] {
            let mut plan = mixed_row(&[Some("a"), Some("b")]);
            plan.styles.apply(key, value);
            assert_eq!(shape(&plan), vec!["fused@0:ab"], "container {key}: {value}");
        }
    }

    /// A declared container with no children has nothing to fuse.
    #[test]
    fn inline_runs_decline_an_empty_container() {
        let mut tree = Tree::new();
        let mut code = Node::new_element("span");
        code.attributes
            .insert(INLINE_TEXT_RUNS_ATTRIBUTE.into(), "1".into());
        let code_id = tree.insert(code);
        let plan = build_render_plan(&tree, code_id).expect("plan");
        assert_eq!(inline_text_runs(&plan), None);
    }

    /// `"0"` and `"false"` turn the declaration off, so a caller can carry
    /// the attribute and decide per frame.
    #[test]
    fn inline_runs_honour_a_negative_declaration() {
        for off in ["0", "false"] {
            let mut tree = Tree::new();
            let mut code = Node::new_element("span");
            code.attributes
                .insert(INLINE_TEXT_RUNS_ATTRIBUTE.into(), off.into());
            let code_id = tree.insert(code);
            let mut span = Node::new_element("span");
            span.styles.insert("color".into(), "#fff".into());
            let span_id = tree.insert(span);
            let text_id = tree.insert(Node::new_text("x"));
            tree.append_child(span_id, text_id);
            tree.append_child(code_id, span_id);
            let plan = build_render_plan(&tree, code_id).expect("plan");
            assert_eq!(inline_text_runs(&plan), None, "attribute value {off:?}");
        }
    }

    #[test]
    fn test_classify_div() {
        assert_eq!(classify_tag("div"), GpuiElementKind::Div);
    }

    #[test]
    fn test_classify_root() {
        assert_eq!(classify_tag("root"), GpuiElementKind::Div);
    }

    #[test]
    fn test_classify_button() {
        assert_eq!(classify_tag("button"), GpuiElementKind::Div);
    }

    #[test]
    fn test_classify_span() {
        assert_eq!(classify_tag("span"), GpuiElementKind::TextContainer);
    }

    #[test]
    fn test_classify_p() {
        assert_eq!(classify_tag("p"), GpuiElementKind::TextContainer);
    }

    #[test]
    fn test_classify_h1() {
        assert_eq!(classify_tag("h1"), GpuiElementKind::TextContainer);
    }

    #[test]
    fn test_classify_img() {
        assert_eq!(classify_tag("img"), GpuiElementKind::Img);
    }

    #[test]
    fn test_classify_svg() {
        assert_eq!(classify_tag("svg"), GpuiElementKind::Svg);
    }

    #[test]
    fn test_classify_unknown_defaults_to_div() {
        assert_eq!(classify_tag("custom-component"), GpuiElementKind::Div);
    }

    #[test]
    fn test_classify_text_node() {
        let node = Node::new_text("hello");
        assert_eq!(classify_node(&node), GpuiElementKind::TextNode);
    }

    #[test]
    fn test_classify_element_node() {
        let node = Node::new_element("div");
        assert_eq!(classify_node(&node), GpuiElementKind::Div);
    }

    #[test]
    fn test_margins_are_kept_per_side() {
        let mut node = Node::new_element("div");
        node.styles.insert("margin".into(), "2px".into());
        node.styles.insert("margin-top".into(), "8px".into());
        node.styles.insert("margin-left".into(), "4px".into());
        let styles = GpuiStyles::from_node(&node);
        assert_eq!(styles.m.as_deref(), Some("2px"));
        assert_eq!(styles.margin_top.as_deref(), Some("8px"));
        assert_eq!(styles.margin_left.as_deref(), Some("4px"));
        assert_eq!(styles.margin_right, None);
        assert_eq!(styles.margin_bottom, None);
    }

    #[test]
    fn test_corner_radii_are_kept_per_corner() {
        let mut node = Node::new_element("div");
        node.styles.insert("border-radius".into(), "6px".into());
        node.styles
            .insert("border-bottom-left-radius".into(), "0px".into());
        node.styles
            .insert("border-bottom-right-radius".into(), "0px".into());
        let styles = GpuiStyles::from_node(&node);
        assert_eq!(styles.rounded.as_deref(), Some("6px"));
        assert_eq!(styles.rounded_bl.as_deref(), Some("0px"));
        assert_eq!(styles.rounded_br.as_deref(), Some("0px"));
        assert_eq!(styles.rounded_tl, None);
        assert_eq!(styles.rounded_tr, None);
    }

    #[test]
    fn test_styles_from_node_css_properties() {
        let mut node = Node::new_element("div");
        node.styles.insert("width".into(), "100%".into());
        node.styles.insert("height".into(), "50px".into());
        node.styles
            .insert("background-color".into(), "red".into());
        node.styles.insert("font-size".into(), "16".into());
        node.styles.insert("border-radius".into(), "8".into());
        node.styles.insert("color".into(), "#ffffff".into());
        node.styles.insert("gap".into(), "10".into());
        node.styles.insert("padding".into(), "5".into());
        node.styles.insert("margin".into(), "3".into());

        let styles = GpuiStyles::from_node(&node);
        assert_eq!(styles.w.as_deref(), Some("100%"));
        assert_eq!(styles.h.as_deref(), Some("50px"));
        assert_eq!(styles.bg.as_deref(), Some("red"));
        assert_eq!(styles.text_size.as_deref(), Some("16"));
        assert_eq!(styles.rounded.as_deref(), Some("8"));
        assert_eq!(styles.text_color.as_deref(), Some("#ffffff"));
        assert_eq!(styles.gap.as_deref(), Some("10"));
        assert_eq!(styles.p.as_deref(), Some("5"));
        assert_eq!(styles.m.as_deref(), Some("3"));
    }

    #[test]
    fn test_styles_per_side_and_placement_keys() {
        // The keys the window draws since 2026-09-29: per-side borders and
        // padding, absolute insets and a grow factor — each parsed into its
        // own field, not dropped as unknown.
        let mut node = Node::new_element("div");
        for (k, v) in [
            ("border-left-width", "3px"),
            ("border-top-width", "1px"),
            ("padding-left", "10px"),
            ("padding-bottom", "2px"),
            ("flex-grow", "1"),
            ("position", "absolute"),
            ("top", "30px"),
            ("left", "40px"),
        ] {
            node.styles.insert(k.into(), v.into());
        }
        let styles = GpuiStyles::from_node(&node);
        assert_eq!(styles.border_left_width.as_deref(), Some("3px"));
        assert_eq!(styles.border_top_width.as_deref(), Some("1px"));
        assert_eq!(styles.border_right_width, None);
        assert_eq!(styles.padding_left.as_deref(), Some("10px"));
        assert_eq!(styles.padding_bottom.as_deref(), Some("2px"));
        assert_eq!(styles.padding_top, None);
        assert_eq!(styles.flex_grow.as_deref(), Some("1"));
        assert_eq!(styles.top.as_deref(), Some("30px"));
        assert_eq!(styles.left.as_deref(), Some("40px"));
    }

    #[test]
    fn test_styles_text_decoration() {
        // A menu's access key is underlined (`text-decoration: underline`);
        // the key is parsed into its own field, and an element that names no
        // decoration has none.
        let mut node = Node::new_element("span");
        node.styles.insert("text-decoration".into(), "underline".into());
        assert_eq!(
            GpuiStyles::from_node(&node).text_decoration.as_deref(),
            Some("underline")
        );
        let plain = Node::new_element("span");
        assert_eq!(GpuiStyles::from_node(&plain).text_decoration, None);
    }

    /// `letter-spacing` IS NOT PARSED, because nothing draws it: `gpui-pre`
    /// 0.3.5 has no letter-spacing, and no element builder in this shim
    /// spaces glyphs by hand. A property the shim records but never applies
    /// makes the render plan disagree with the window, so it falls through to
    /// the unknown-property arm instead (`docs/shim-design.md`, "Only drawn
    /// properties are parsed"; issue
    /// `2026-10-17-letter-spacing-is-parsed-and-reported-but-never-applied.md`).
    ///
    /// Compared against the DEFAULT rather than a named field, so the test
    /// also fails if the value were stored under any other field. Both
    /// spellings the parser used to accept are checked.
    #[test]
    fn test_styles_letter_spacing_is_not_recorded() {
        for key in ["letter-spacing", "letter_spacing"] {
            let mut node = Node::new_element("span");
            node.styles.insert(key.into(), "2px".into());
            assert_eq!(
                GpuiStyles::from_node(&node),
                GpuiStyles::default(),
                "`{key}` must be ignored as unknown, not recorded"
            );
        }
        // Positive control: a property the shim does draw on the same node
        // shape is recorded, so the comparison above can fail.
        let mut drawn = Node::new_element("span");
        drawn.styles.insert("line-height".into(), "20px".into());
        assert_ne!(GpuiStyles::from_node(&drawn), GpuiStyles::default());
    }

    #[test]
    fn test_styles_flex_direction_normalization() {
        let mut node = Node::new_element("div");
        node.styles.insert("flex-direction".into(), "row".into());
        let styles = GpuiStyles::from_node(&node);
        assert_eq!(styles.flex_direction.as_deref(), Some("row"));

        let mut node2 = Node::new_element("div");
        node2.styles.insert("flex-direction".into(), "column".into());
        let styles2 = GpuiStyles::from_node(&node2);
        assert_eq!(styles2.flex_direction.as_deref(), Some("column"));

        let mut node3 = Node::new_element("div");
        node3.styles.insert("flex-direction".into(), "col".into());
        let styles3 = GpuiStyles::from_node(&node3);
        assert_eq!(styles3.flex_direction.as_deref(), Some("column"));
    }

    #[test]
    fn test_styles_alignment_normalization() {
        let mut node = Node::new_element("div");
        node.styles.insert("align-items".into(), "center".into());
        node.styles
            .insert("justify-content".into(), "space-between".into());
        let styles = GpuiStyles::from_node(&node);
        assert_eq!(styles.items.as_deref(), Some("center"));
        assert_eq!(styles.justify.as_deref(), Some("space_between"));
    }

    #[test]
    fn test_styles_attribute_fallback() {
        let mut node = Node::new_element("div");
        node.attributes.insert("width".into(), "200".into());
        node.attributes.insert("height".into(), "100".into());
        let styles = GpuiStyles::from_node(&node);
        assert_eq!(styles.w.as_deref(), Some("200"));
        assert_eq!(styles.h.as_deref(), Some("100"));
    }

    #[test]
    fn test_styles_explicit_style_overrides_attribute() {
        let mut node = Node::new_element("div");
        node.styles.insert("width".into(), "300".into());
        node.attributes.insert("width".into(), "200".into());
        let styles = GpuiStyles::from_node(&node);
        assert_eq!(styles.w.as_deref(), Some("300"));
    }

    #[test]
    fn test_styles_internal_attributes_ignored() {
        let mut node = Node::new_element("div");
        node.attributes
            .insert("__text_content".into(), "hello".into());
        let styles = GpuiStyles::from_node(&node);
        assert!(styles.w.is_none());
        assert!(styles.bg.is_none());
    }

    #[test]
    fn test_build_render_plan_single_node() {
        let mut tree = Tree::new();
        let node = Node::new_element("div");
        let id = tree.insert(node);

        let plan = build_render_plan(&tree, id).unwrap();
        assert_eq!(plan.node_id, id.0);
        assert_eq!(plan.kind, GpuiElementKind::Div);
        assert_eq!(plan.tag, "div");
        assert!(plan.children.is_empty());
        assert!(plan.text.is_none());
        assert!(!plan.has_click_handler);
    }

    #[test]
    fn test_build_render_plan_text_node() {
        let mut tree = Tree::new();
        let node = Node::new_text("hello world");
        let id = tree.insert(node);

        let plan = build_render_plan(&tree, id).unwrap();
        assert_eq!(plan.kind, GpuiElementKind::TextNode);
        assert_eq!(plan.tag, "#text");
        assert_eq!(plan.text.as_deref(), Some("hello world"));
    }

    #[test]
    fn test_build_render_plan_with_children() {
        let mut tree = Tree::new();
        let root = Node::new_element("root");
        let child1 = Node::new_element("span");
        let child2 = Node::new_text("text");

        let root_id = tree.insert(root);
        let c1_id = tree.insert(child1);
        let c2_id = tree.insert(child2);

        tree.append_child(root_id, c1_id);
        tree.append_child(root_id, c2_id);

        let plan = build_render_plan(&tree, root_id).unwrap();
        assert_eq!(plan.children.len(), 2);
        assert_eq!(plan.children[0].kind, GpuiElementKind::TextContainer);
        assert_eq!(plan.children[1].kind, GpuiElementKind::TextNode);
        assert_eq!(plan.children[1].text.as_deref(), Some("text"));
    }

    #[test]
    fn test_build_render_plan_with_styles() {
        let mut tree = Tree::new();
        let mut node = Node::new_element("div");
        node.styles.insert("width".into(), "100%".into());
        node.styles.insert("background".into(), "red".into());
        let id = tree.insert(node);

        let plan = build_render_plan(&tree, id).unwrap();
        assert_eq!(plan.styles.w.as_deref(), Some("100%"));
        assert_eq!(plan.styles.bg.as_deref(), Some("red"));
    }

    #[test]
    fn test_build_render_plan_with_click_handler() {
        let mut tree = Tree::new();
        let mut node = Node::new_element("button");
        extern "C" fn noop(_p: *const crate::input::GpuiEventPayload) {}
        node.event_listeners
            .entry("click".into())
            .or_default()
            .push(EventListener { callback: noop, callback_id: 0 });
        let id = tree.insert(node);

        let plan = build_render_plan(&tree, id).unwrap();
        assert!(plan.has_click_handler);
        assert_eq!(plan.kind, GpuiElementKind::Div);
    }

    #[test]
    fn test_build_render_plan_nonexistent_node() {
        let tree = Tree::new();
        let plan = build_render_plan(&tree, NodeId(999));
        assert!(plan.is_none());
    }

    #[test]
    fn test_build_render_plan_deep_tree() {
        let mut tree = Tree::new();
        let root = Node::new_element("root");
        let div = Node::new_element("div");
        let span = Node::new_element("span");
        let text = Node::new_text("nested");

        let root_id = tree.insert(root);
        let div_id = tree.insert(div);
        let span_id = tree.insert(span);
        let text_id = tree.insert(text);

        tree.append_child(root_id, div_id);
        tree.append_child(div_id, span_id);
        tree.append_child(span_id, text_id);

        let plan = build_render_plan(&tree, root_id).unwrap();
        assert_eq!(count_render_nodes(&plan), 4);
        assert_eq!(plan.children.len(), 1); // div
        assert_eq!(plan.children[0].children.len(), 1); // span
        assert_eq!(plan.children[0].children[0].children.len(), 1); // text
        assert_eq!(
            plan.children[0].children[0].children[0].text.as_deref(),
            Some("nested")
        );
    }

    #[test]
    fn test_build_render_plan_element_with_text_content() {
        let mut tree = Tree::new();
        let mut node = Node::new_element("div");
        node.set_text_content("direct text");
        let id = tree.insert(node);

        let plan = build_render_plan(&tree, id).unwrap();
        assert_eq!(plan.text.as_deref(), Some("direct text"));
    }

    #[test]
    fn test_count_render_nodes() {
        let plan = RenderNode {
            node_id: 1,
            kind: GpuiElementKind::Div,
            tag: "div".into(),
            styles: GpuiStyles::default(),
            text: None,
            has_click_handler: false,
            has_input_handler: false,
            event_names: vec![],
                attributes: BTreeMap::new(),
            children: vec![
                RenderNode {
                    node_id: 2,
                    kind: GpuiElementKind::TextContainer,
                    tag: "span".into(),
                    styles: GpuiStyles::default(),
                    text: Some("a".into()),
                    has_click_handler: false,
                    has_input_handler: false,
                    event_names: vec![],
                attributes: BTreeMap::new(),
                    children: vec![],
                },
                RenderNode {
                    node_id: 3,
                    kind: GpuiElementKind::Div,
                    tag: "div".into(),
                    styles: GpuiStyles::default(),
                    text: None,
                    has_click_handler: false,
                    has_input_handler: false,
                    event_names: vec![],
                attributes: BTreeMap::new(),
                    children: vec![RenderNode {
                        node_id: 4,
                        kind: GpuiElementKind::TextNode,
                        tag: "#text".into(),
                        styles: GpuiStyles::default(),
                        text: Some("b".into()),
                        has_click_handler: false,
                        has_input_handler: false,
                        event_names: vec![],
                attributes: BTreeMap::new(),
                        children: vec![],
                    }],
                },
            ],
        };
        assert_eq!(count_render_nodes(&plan), 4);
    }

    #[test]
    fn test_build_render_plan_with_input_handler() {
        let mut tree = Tree::new();
        let mut node = Node::new_element("div");
        extern "C" fn noop(_p: *const crate::input::GpuiEventPayload) {}
        node.event_listeners
            .entry("input".into())
            .or_default()
            .push(EventListener { callback: noop, callback_id: 0 });
        let id = tree.insert(node);

        let plan = build_render_plan(&tree, id).unwrap();
        assert!(plan.has_input_handler);
        assert!(!plan.has_click_handler);
        assert!(plan.event_names.contains(&"input".to_string()));
    }

    #[test]
    fn test_build_render_plan_multiple_event_handlers() {
        let mut tree = Tree::new();
        let mut node = Node::new_element("button");
        extern "C" fn noop(_p: *const crate::input::GpuiEventPayload) {}
        node.event_listeners
            .entry("click".into())
            .or_default()
            .push(EventListener { callback: noop, callback_id: 0 });
        node.event_listeners
            .entry("input".into())
            .or_default()
            .push(EventListener { callback: noop, callback_id: 0 });
        node.event_listeners
            .entry("hover".into())
            .or_default()
            .push(EventListener { callback: noop, callback_id: 0 });
        let id = tree.insert(node);

        let plan = build_render_plan(&tree, id).unwrap();
        assert!(plan.has_click_handler);
        assert!(plan.has_input_handler);
        assert_eq!(plan.event_names.len(), 3);
        assert!(plan.event_names.contains(&"click".to_string()));
        assert!(plan.event_names.contains(&"input".to_string()));
        assert!(plan.event_names.contains(&"hover".to_string()));
    }

    #[test]
    fn test_build_render_plan_no_handlers_by_default() {
        let mut tree = Tree::new();
        let node = Node::new_element("div");
        let id = tree.insert(node);

        let plan = build_render_plan(&tree, id).unwrap();
        assert!(!plan.has_click_handler);
        assert!(!plan.has_input_handler);
        assert!(plan.event_names.is_empty());
    }

    #[test]
    fn test_render_plan_preserves_event_handlers_in_children() {
        let mut tree = Tree::new();
        extern "C" fn noop(_p: *const crate::input::GpuiEventPayload) {}

        let root = Node::new_element("root");
        let root_id = tree.insert(root);

        let mut btn = Node::new_element("button");
        btn.event_listeners
            .entry("click".into())
            .or_default()
            .push(EventListener { callback: noop, callback_id: 0 });
        let btn_id = tree.insert(btn);

        let mut input = Node::new_element("div");
        input
            .event_listeners
            .entry("input".into())
            .or_default()
            .push(EventListener { callback: noop, callback_id: 0 });
        let input_id = tree.insert(input);

        tree.append_child(root_id, btn_id);
        tree.append_child(root_id, input_id);

        let plan = build_render_plan(&tree, root_id).unwrap();
        assert!(!plan.has_click_handler);
        assert!(!plan.has_input_handler);
        assert_eq!(plan.children.len(), 2);
        assert!(plan.children[0].has_click_handler);
        assert!(!plan.children[0].has_input_handler);
        assert!(!plan.children[1].has_click_handler);
        assert!(plan.children[1].has_input_handler);
    }

    #[test]
    fn test_repaint_flag_with_render_plan_rebuild() {
        let mut tree = Tree::new();
        let root = Node::new_element("root");
        let root_id = tree.insert(root);

        let label = Node::new_element("span");
        let label_id = tree.insert(label);
        tree.append_child(root_id, label_id);

        let plan1 = build_render_plan(&tree, root_id).unwrap();
        assert_eq!(plan1.children.len(), 1);

        let label2 = Node::new_element("span");
        let label2_id = tree.insert(label2);
        tree.append_child(root_id, label2_id);

        let plan2 = build_render_plan(&tree, root_id).unwrap();
        assert_eq!(plan2.children.len(), 2);
    }

    #[test]
    fn test_html_semantic_tags_classify_correctly() {
        // Container elements -> Div
        for tag in &[
            "section", "header", "footer", "nav", "main", "article", "aside", "form", "fieldset",
        ] {
            assert_eq!(
                classify_tag(tag),
                GpuiElementKind::Div,
                "Expected {tag} to classify as Div"
            );
        }
        // Text-oriented elements -> TextContainer
        for tag in &[
            "a", "strong", "em", "code", "h1", "h2", "h3", "h4", "h5", "h6",
        ] {
            assert_eq!(
                classify_tag(tag),
                GpuiElementKind::TextContainer,
                "Expected {tag} to classify as TextContainer"
            );
        }
        // Block text tags -> TextContainer (GPUI maps these the same)
        for tag in &["pre"] {
            assert_eq!(
                classify_tag(tag),
                GpuiElementKind::TextContainer,
                "Expected {tag} to classify as TextContainer"
            );
        }
    }

    #[test]
    fn test_render_plan_tag_preserved() {
        let mut tree = Tree::new();
        let node = Node::new_element("button");
        let id = tree.insert(node);
        let plan = build_render_plan(&tree, id).unwrap();
        assert_eq!(plan.tag, "button");
        assert_eq!(plan.kind, GpuiElementKind::Div);
    }
}

/// Integration tests that exercise the full FFI -> shadow tree -> render plan pipeline.
#[cfg(test)]
#[allow(unused_unsafe)]
mod integration_tests {
    use super::*;
    use crate::{
        gpui_add_event_listener, gpui_append_child, gpui_create_element,
        gpui_create_text_node, gpui_destroy_element, gpui_dispatch_event,
        gpui_remove_child, gpui_render_plan_element_count, gpui_render_plan_json,
        gpui_free_string, gpui_reset_tree, gpui_set_attribute, gpui_set_style,
        gpui_set_text_content, gpui_verify_render_plan, lock_tree,
        GpuiElement as GpuiHandle,
    };
    use serial_test::serial;
    use std::ffi::{CStr, CString};
    use std::sync::atomic::{AtomicU32, Ordering};

    fn c(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    /// Helper: get the render plan as a parsed JSON value via the FFI function.
    fn get_plan_json(root: *mut GpuiHandle) -> serde_json::Value {
        unsafe {
            let json_ptr = gpui_render_plan_json(root);
            assert!(!json_ptr.is_null(), "render plan JSON should not be null");
            let json_str = CStr::from_ptr(json_ptr).to_str().unwrap().to_string();
            gpui_free_string(json_ptr);
            serde_json::from_str(&json_str).expect("render plan should be valid JSON")
        }
    }

    /// Helper: get the render plan directly (Tree API, not FFI).
    fn get_plan_direct(root: *mut GpuiHandle) -> RenderNode {
        let node_id = crate::tree::NodeId(unsafe { (*root).node_id });
        let tree = lock_tree();
        build_render_plan(&tree, node_id).expect("render plan should exist")
    }

    // ===================================================================
    // Full pipeline: counter app
    // ===================================================================

    #[test]
    #[serial]
    fn test_full_pipeline_counter_app() {
        unsafe {
            gpui_reset_tree();

            let root = gpui_create_element(c("div").as_ptr());
            let label = gpui_create_element(c("span").as_ptr());
            let text = gpui_create_text_node(c("Count: 0").as_ptr());
            gpui_append_child(label, text);
            gpui_append_child(root, label);

            let button = gpui_create_element(c("button").as_ptr());
            let btn_text = gpui_create_text_node(c("+").as_ptr());
            gpui_append_child(button, btn_text);

            extern "C" fn on_click(_p: *const crate::input::GpuiEventPayload) {}
            gpui_add_event_listener(button, c("click").as_ptr(), on_click);
            gpui_append_child(root, button);

            // Verify via direct render plan (Tree API)
            let plan = get_plan_direct(root);
            assert_eq!(plan.kind, GpuiElementKind::Div); // div -> Div
            assert_eq!(plan.children.len(), 2);

            // span -> TextContainer
            assert_eq!(plan.children[0].kind, GpuiElementKind::TextContainer);
            // text node -> TextNode with text
            assert_eq!(plan.children[0].children[0].kind, GpuiElementKind::TextNode);
            assert_eq!(plan.children[0].children[0].text.as_deref(), Some("Count: 0"));

            // button -> Div with click handler
            assert_eq!(plan.children[1].kind, GpuiElementKind::Div);
            assert!(plan.children[1].has_click_handler);
            assert_eq!(plan.children[1].children[0].text.as_deref(), Some("+"));

            // Verify via JSON FFI (the path Nim uses)
            let json = get_plan_json(root);
            assert_eq!(json["kind"], "Div");
            assert_eq!(json["children"].as_array().unwrap().len(), 2);
            assert_eq!(json["children"][0]["kind"], "TextContainer");
            assert_eq!(json["children"][0]["children"][0]["text"], "Count: 0");
            assert_eq!(json["children"][1]["kind"], "Div");
            assert_eq!(json["children"][1]["has_click_handler"], true);

            // Element count
            assert_eq!(gpui_render_plan_element_count(root), 5);
            assert_eq!(gpui_verify_render_plan(root), 1);

            gpui_destroy_element(root);
            gpui_destroy_element(label);
            gpui_destroy_element(text);
            gpui_destroy_element(button);
            gpui_destroy_element(btn_text);
        }
    }

    // ===================================================================
    // Style propagation through render plan
    // ===================================================================

    #[test]
    #[serial]
    fn test_render_plan_styles_via_ffi() {
        unsafe {
            gpui_reset_tree();

            let div = gpui_create_element(c("div").as_ptr());
            gpui_set_style(div, c("background-color").as_ptr(), c("red").as_ptr());
            gpui_set_style(div, c("width").as_ptr(), c("200px").as_ptr());
            gpui_set_style(div, c("flex-direction").as_ptr(), c("row").as_ptr());
            gpui_set_style(div, c("font-size").as_ptr(), c("16").as_ptr());
            gpui_set_style(div, c("border-radius").as_ptr(), c("8").as_ptr());
            gpui_set_style(div, c("padding").as_ptr(), c("10").as_ptr());

            let plan = get_plan_direct(div);
            assert_eq!(plan.styles.bg.as_deref(), Some("red"));
            assert_eq!(plan.styles.w.as_deref(), Some("200px"));
            assert_eq!(plan.styles.flex_direction.as_deref(), Some("row"));
            assert_eq!(plan.styles.text_size.as_deref(), Some("16"));
            assert_eq!(plan.styles.rounded.as_deref(), Some("8"));
            assert_eq!(plan.styles.p.as_deref(), Some("10"));

            // Also verify via JSON
            let json = get_plan_json(div);
            assert_eq!(json["styles"]["bg"], "red");
            assert_eq!(json["styles"]["w"], "200px");
            assert_eq!(json["styles"]["flex_direction"], "row");

            gpui_destroy_element(div);
        }
    }

    #[test]
    #[serial]
    fn test_render_plan_styles_column_direction() {
        unsafe {
            gpui_reset_tree();
            let div = gpui_create_element(c("div").as_ptr());
            gpui_set_style(div, c("flex-direction").as_ptr(), c("column").as_ptr());
            let plan = get_plan_direct(div);
            assert_eq!(plan.styles.flex_direction.as_deref(), Some("column"));
            gpui_destroy_element(div);
        }
    }

    #[test]
    #[serial]
    fn test_render_plan_styles_from_attributes_fallback() {
        unsafe {
            gpui_reset_tree();
            let div = gpui_create_element(c("div").as_ptr());
            gpui_set_attribute(div, c("width").as_ptr(), c("300").as_ptr());
            gpui_set_attribute(div, c("height").as_ptr(), c("200").as_ptr());
            let plan = get_plan_direct(div);
            assert_eq!(plan.styles.w.as_deref(), Some("300"));
            assert_eq!(plan.styles.h.as_deref(), Some("200"));
            gpui_destroy_element(div);
        }
    }

    // ===================================================================
    // Event handler wiring through render plan
    // ===================================================================

    #[test]
    #[serial]
    fn test_render_plan_event_handlers_via_ffi() {
        unsafe {
            gpui_reset_tree();

            static CLICKED: AtomicU32 = AtomicU32::new(0);
            extern "C" fn on_click(_p: *const crate::input::GpuiEventPayload) {
                CLICKED.fetch_add(1, Ordering::SeqCst);
            }
            CLICKED.store(0, Ordering::SeqCst);

            let button = gpui_create_element(c("button").as_ptr());
            gpui_add_event_listener(button, c("click").as_ptr(), on_click);

            let plan = get_plan_direct(button);
            assert!(plan.has_click_handler);
            assert!(!plan.has_input_handler);

            // Dispatch through the shadow tree -- verify callback fires
            gpui_dispatch_event(button, c("click").as_ptr());
            assert_eq!(CLICKED.load(Ordering::SeqCst), 1);

            gpui_destroy_element(button);
        }
    }

    #[test]
    #[serial]
    fn test_render_plan_multiple_event_types_via_ffi() {
        unsafe {
            gpui_reset_tree();
            extern "C" fn noop(_p: *const crate::input::GpuiEventPayload) {}

            let node = gpui_create_element(c("div").as_ptr());
            gpui_add_event_listener(node, c("click").as_ptr(), noop);
            gpui_add_event_listener(node, c("input").as_ptr(), noop);
            gpui_add_event_listener(node, c("hover").as_ptr(), noop);

            let plan = get_plan_direct(node);
            assert!(plan.has_click_handler);
            assert!(plan.has_input_handler);
            assert_eq!(plan.event_names.len(), 3);
            assert!(plan.event_names.contains(&"click".to_string()));
            assert!(plan.event_names.contains(&"input".to_string()));
            assert!(plan.event_names.contains(&"hover".to_string()));

            gpui_destroy_element(node);
        }
    }

    #[test]
    #[serial]
    fn test_render_plan_no_handlers_by_default_via_ffi() {
        unsafe {
            gpui_reset_tree();
            let div = gpui_create_element(c("div").as_ptr());
            let plan = get_plan_direct(div);
            assert!(!plan.has_click_handler);
            assert!(!plan.has_input_handler);
            assert!(plan.event_names.is_empty());
            gpui_destroy_element(div);
        }
    }

    // ===================================================================
    // Full demo app render plan (task manager)
    // ===================================================================

    #[test]
    #[serial]
    fn test_full_demo_app_render_plan() {
        unsafe {
            gpui_reset_tree();

            let app = gpui_create_element(c("div").as_ptr());

            // Header
            let header = gpui_create_element(c("header").as_ptr());
            let title = gpui_create_element(c("h1").as_ptr());
            let title_text = gpui_create_text_node(c("Task Manager").as_ptr());
            gpui_append_child(title, title_text);
            gpui_append_child(header, title);
            gpui_append_child(app, header);

            // Input area
            let input_area = gpui_create_element(c("div").as_ptr());
            let input = gpui_create_element(c("input").as_ptr());
            let add_btn = gpui_create_element(c("button").as_ptr());
            let add_text = gpui_create_text_node(c("Add").as_ptr());
            extern "C" fn on_add(_p: *const crate::input::GpuiEventPayload) {}
            gpui_add_event_listener(add_btn, c("click").as_ptr(), on_add);
            gpui_append_child(add_btn, add_text);
            gpui_append_child(input_area, input);
            gpui_append_child(input_area, add_btn);
            gpui_append_child(app, input_area);

            // Task list
            let task_list = gpui_create_element(c("ul").as_ptr());
            let mut task_handles = Vec::new();
            for task_name in &["Design API", "Write tests"] {
                let li = gpui_create_element(c("li").as_ptr());
                let span = gpui_create_element(c("span").as_ptr());
                let text = gpui_create_text_node(c(task_name).as_ptr());
                gpui_append_child(span, text);
                gpui_append_child(li, span);
                gpui_append_child(task_list, li);
                task_handles.push((li, span, text));
            }
            gpui_append_child(app, task_list);

            // Footer
            let footer = gpui_create_element(c("footer").as_ptr());
            let count_span = gpui_create_element(c("span").as_ptr());
            let count_text = gpui_create_text_node(c("2 tasks").as_ptr());
            gpui_append_child(count_span, count_text);
            gpui_append_child(footer, count_span);
            gpui_append_child(app, footer);

            // Verify render plan
            let plan = get_plan_direct(app);

            assert_eq!(plan.kind, GpuiElementKind::Div);
            assert_eq!(plan.children.len(), 4);

            // header -> Div
            assert_eq!(plan.children[0].kind, GpuiElementKind::Div);
            // h1 -> TextContainer
            assert_eq!(plan.children[0].children[0].kind, GpuiElementKind::TextContainer);

            // input area -> Div with 2 children
            assert_eq!(plan.children[1].kind, GpuiElementKind::Div);
            assert_eq!(plan.children[1].children.len(), 2);
            // add button -> Div with click handler
            assert!(plan.children[1].children[1].has_click_handler);

            // task list -> Div with 2 children
            assert_eq!(plan.children[2].kind, GpuiElementKind::Div);
            assert_eq!(plan.children[2].children.len(), 2);

            // First task: li -> Div > span -> TextContainer > text
            assert_eq!(plan.children[2].children[0].kind, GpuiElementKind::Div);
            assert_eq!(
                plan.children[2].children[0].children[0].kind,
                GpuiElementKind::TextContainer
            );
            assert_eq!(
                plan.children[2].children[0].children[0].children[0]
                    .text
                    .as_deref(),
                Some("Design API")
            );

            // Second task
            assert_eq!(
                plan.children[2].children[1].children[0].children[0]
                    .text
                    .as_deref(),
                Some("Write tests")
            );

            // footer -> Div > span -> TextContainer
            assert_eq!(plan.children[3].kind, GpuiElementKind::Div);
            assert_eq!(
                plan.children[3].children[0].kind,
                GpuiElementKind::TextContainer
            );
            assert_eq!(
                plan.children[3].children[0].children[0].text.as_deref(),
                Some("2 tasks")
            );

            // Total element count
            assert_eq!(gpui_render_plan_element_count(app), 18);

            // Clean up
            gpui_destroy_element(app);
            gpui_destroy_element(header);
            gpui_destroy_element(title);
            gpui_destroy_element(title_text);
            gpui_destroy_element(input_area);
            gpui_destroy_element(input);
            gpui_destroy_element(add_btn);
            gpui_destroy_element(add_text);
            gpui_destroy_element(task_list);
            gpui_destroy_element(footer);
            gpui_destroy_element(count_span);
            gpui_destroy_element(count_text);
            for (li, span, text) in task_handles {
                gpui_destroy_element(li);
                gpui_destroy_element(span);
                gpui_destroy_element(text);
            }
        }
    }

    // ===================================================================
    // Render plan after mutations
    // ===================================================================

    #[test]
    #[serial]
    fn test_render_plan_after_adding_child() {
        unsafe {
            gpui_reset_tree();
            let root = gpui_create_element(c("div").as_ptr());
            let label1 = gpui_create_element(c("span").as_ptr());
            gpui_append_child(root, label1);

            let plan1 = get_plan_direct(root);
            assert_eq!(plan1.children.len(), 1);
            assert_eq!(gpui_render_plan_element_count(root), 2);

            let label2 = gpui_create_element(c("span").as_ptr());
            gpui_append_child(root, label2);

            let plan2 = get_plan_direct(root);
            assert_eq!(plan2.children.len(), 2);
            assert_eq!(gpui_render_plan_element_count(root), 3);

            gpui_destroy_element(root);
            gpui_destroy_element(label1);
            gpui_destroy_element(label2);
        }
    }

    #[test]
    #[serial]
    fn test_render_plan_after_removing_child() {
        unsafe {
            gpui_reset_tree();
            let root = gpui_create_element(c("div").as_ptr());
            let c1 = gpui_create_element(c("span").as_ptr());
            let c2 = gpui_create_element(c("span").as_ptr());
            gpui_append_child(root, c1);
            gpui_append_child(root, c2);
            assert_eq!(gpui_render_plan_element_count(root), 3);

            gpui_remove_child(root, c1);
            let plan = get_plan_direct(root);
            assert_eq!(plan.children.len(), 1);
            assert_eq!(gpui_render_plan_element_count(root), 2);

            gpui_destroy_element(root);
            gpui_destroy_element(c1);
            gpui_destroy_element(c2);
        }
    }

    #[test]
    #[serial]
    fn test_render_plan_after_text_update() {
        unsafe {
            gpui_reset_tree();
            let root = gpui_create_element(c("div").as_ptr());
            let text_node = gpui_create_text_node(c("initial").as_ptr());
            gpui_append_child(root, text_node);

            let plan1 = get_plan_direct(root);
            assert_eq!(plan1.children[0].text.as_deref(), Some("initial"));

            gpui_set_text_content(text_node, c("updated").as_ptr());

            let plan2 = get_plan_direct(root);
            assert_eq!(plan2.children[0].text.as_deref(), Some("updated"));

            gpui_destroy_element(root);
            gpui_destroy_element(text_node);
        }
    }

    #[test]
    #[serial]
    fn test_render_plan_after_style_change() {
        unsafe {
            gpui_reset_tree();
            let div = gpui_create_element(c("div").as_ptr());
            gpui_set_style(div, c("background").as_ptr(), c("red").as_ptr());

            let plan1 = get_plan_direct(div);
            assert_eq!(plan1.styles.bg.as_deref(), Some("red"));

            gpui_set_style(div, c("background").as_ptr(), c("blue").as_ptr());

            let plan2 = get_plan_direct(div);
            assert_eq!(plan2.styles.bg.as_deref(), Some("blue"));

            gpui_destroy_element(div);
        }
    }

    #[test]
    #[serial]
    fn test_render_plan_after_adding_event_handler() {
        unsafe {
            gpui_reset_tree();
            let div = gpui_create_element(c("div").as_ptr());

            let plan1 = get_plan_direct(div);
            assert!(!plan1.has_click_handler);

            extern "C" fn noop(_p: *const crate::input::GpuiEventPayload) {}
            gpui_add_event_listener(div, c("click").as_ptr(), noop);

            let plan2 = get_plan_direct(div);
            assert!(plan2.has_click_handler);

            gpui_destroy_element(div);
        }
    }

    // ===================================================================
    // Tag classification through FFI -> render plan
    // ===================================================================

    #[test]
    #[serial]
    fn test_tag_classification_through_ffi() {
        let test_cases: Vec<(&str, GpuiElementKind)> = vec![
            ("div", GpuiElementKind::Div),
            ("section", GpuiElementKind::Div),
            ("header", GpuiElementKind::Div),
            ("footer", GpuiElementKind::Div),
            ("button", GpuiElementKind::Div),
            ("span", GpuiElementKind::TextContainer),
            ("label", GpuiElementKind::TextContainer),
            ("h1", GpuiElementKind::TextContainer),
            ("a", GpuiElementKind::TextContainer),
            ("strong", GpuiElementKind::TextContainer),
            ("p", GpuiElementKind::TextContainer),
            ("pre", GpuiElementKind::TextContainer),
            ("img", GpuiElementKind::Img),
            ("svg", GpuiElementKind::Svg),
        ];

        for (tag, expected_kind) in test_cases {
            unsafe {
                gpui_reset_tree();
                let elem = gpui_create_element(c(tag).as_ptr());
                let plan = get_plan_direct(elem);
                assert_eq!(
                    plan.kind, expected_kind,
                    "Tag '{}' should classify as {:?}",
                    tag, expected_kind
                );
                gpui_destroy_element(elem);
            }
        }
    }

    #[test]
    #[serial]
    fn test_text_node_classifies_as_text_node_via_ffi() {
        unsafe {
            gpui_reset_tree();
            let text = gpui_create_text_node(c("hello").as_ptr());
            let plan = get_plan_direct(text);
            assert_eq!(plan.kind, GpuiElementKind::TextNode);
            assert_eq!(plan.text.as_deref(), Some("hello"));
            gpui_destroy_element(text);
        }
    }

    // ===================================================================
    // Null / edge cases
    // ===================================================================

    #[test]
    #[serial]
    fn test_render_plan_null_handle() {
        unsafe {
            let json = gpui_render_plan_json(std::ptr::null_mut());
            assert!(json.is_null());
            assert_eq!(gpui_render_plan_element_count(std::ptr::null_mut()), 0);
            assert_eq!(gpui_verify_render_plan(std::ptr::null_mut()), 0);
        }
    }

    // ===================================================================
    // Child handlers preserved in plan
    // ===================================================================

    #[test]
    #[serial]
    fn test_render_plan_child_handlers_preserved() {
        unsafe {
            gpui_reset_tree();
            extern "C" fn on_click(_p: *const crate::input::GpuiEventPayload) {}
            extern "C" fn on_input(_p: *const crate::input::GpuiEventPayload) {}

            let root = gpui_create_element(c("div").as_ptr());
            let btn = gpui_create_element(c("button").as_ptr());
            gpui_add_event_listener(btn, c("click").as_ptr(), on_click);
            let input_div = gpui_create_element(c("div").as_ptr());
            gpui_add_event_listener(input_div, c("input").as_ptr(), on_input);

            gpui_append_child(root, btn);
            gpui_append_child(root, input_div);

            let plan = get_plan_direct(root);
            assert!(!plan.has_click_handler);
            assert!(!plan.has_input_handler);
            assert!(plan.children[0].has_click_handler);
            assert!(!plan.children[0].has_input_handler);
            assert!(!plan.children[1].has_click_handler);
            assert!(plan.children[1].has_input_handler);

            gpui_destroy_element(root);
            gpui_destroy_element(btn);
            gpui_destroy_element(input_div);
        }
    }

    // ===================================================================
    // Deep tree render plan
    // ===================================================================

    #[test]
    #[serial]
    fn test_render_plan_deep_nested_tree_via_ffi() {
        unsafe {
            gpui_reset_tree();
            let root = gpui_create_element(c("root").as_ptr());
            let div = gpui_create_element(c("div").as_ptr());
            let span = gpui_create_element(c("span").as_ptr());
            let text = gpui_create_text_node(c("deep").as_ptr());

            gpui_append_child(span, text);
            gpui_append_child(div, span);
            gpui_append_child(root, div);

            let plan = get_plan_direct(root);
            assert_eq!(count_render_nodes(&plan), 4);
            assert_eq!(
                plan.children[0].children[0].children[0].text.as_deref(),
                Some("deep")
            );

            gpui_destroy_element(root);
            gpui_destroy_element(div);
            gpui_destroy_element(span);
            gpui_destroy_element(text);
        }
    }

    // ===================================================================
    // Event dispatch + plan rebuild (simulates reactive update)
    // ===================================================================

    #[test]
    #[serial]
    fn test_event_dispatch_then_rebuild_plan() {
        unsafe {
            gpui_reset_tree();

            static COUNTER: AtomicU32 = AtomicU32::new(0);
            COUNTER.store(0, Ordering::SeqCst);
            extern "C" fn increment(_p: *const crate::input::GpuiEventPayload) {
                COUNTER.fetch_add(1, Ordering::SeqCst);
            }

            let root = gpui_create_element(c("div").as_ptr());
            let btn = gpui_create_element(c("button").as_ptr());
            let label = gpui_create_text_node(c("Count: 0").as_ptr());
            gpui_add_event_listener(btn, c("click").as_ptr(), increment);
            gpui_append_child(root, label);
            gpui_append_child(root, btn);

            // Initial plan
            let plan1 = get_plan_direct(root);
            assert_eq!(plan1.children[0].text.as_deref(), Some("Count: 0"));

            // Simulate click
            gpui_dispatch_event(btn, c("click").as_ptr());
            assert_eq!(COUNTER.load(Ordering::SeqCst), 1);

            // Simulate reactive update
            gpui_set_text_content(label, c("Count: 1").as_ptr());

            // Rebuild plan -- should reflect the mutation
            let plan2 = get_plan_direct(root);
            assert_eq!(plan2.children[0].text.as_deref(), Some("Count: 1"));

            gpui_destroy_element(root);
            gpui_destroy_element(btn);
            gpui_destroy_element(label);
        }
    }

    // ===================================================================
    // JSON serialization round-trip
    // ===================================================================

    #[test]
    #[serial]
    fn test_render_plan_json_round_trip() {
        unsafe {
            gpui_reset_tree();
            let div = gpui_create_element(c("div").as_ptr());
            gpui_set_style(div, c("background").as_ptr(), c("red").as_ptr());
            let span = gpui_create_element(c("span").as_ptr());
            let text = gpui_create_text_node(c("hello").as_ptr());
            gpui_append_child(span, text);
            gpui_append_child(div, span);

            extern "C" fn noop(_p: *const crate::input::GpuiEventPayload) {}
            gpui_add_event_listener(div, c("click").as_ptr(), noop);

            let json = get_plan_json(div);
            assert_eq!(json["kind"], "Div");
            assert_eq!(json["tag"], "div");
            assert_eq!(json["has_click_handler"], true);
            assert_eq!(json["styles"]["bg"], "red");
            assert_eq!(json["children"][0]["kind"], "TextContainer");
            assert_eq!(json["children"][0]["tag"], "span");
            assert_eq!(json["children"][0]["children"][0]["kind"], "TextNode");
            assert_eq!(json["children"][0]["children"][0]["text"], "hello");

            gpui_destroy_element(div);
            gpui_destroy_element(span);
            gpui_destroy_element(text);
        }
    }

    #[test]
    #[serial]
    fn test_render_plan_json_escaping() {
        unsafe {
            gpui_reset_tree();
            let node = gpui_create_text_node(c("hello \"world\"").as_ptr());
            let json = get_plan_json(node);
            assert_eq!(json["text"], "hello \"world\"");
            gpui_destroy_element(node);
        }
    }

    /// A text node carrying CONTROL CHARACTERS still serialises to a document
    /// a strict parser accepts.
    ///
    /// The case above tested quotes and backslashes, which is what the old
    /// hand-written escape chain handled — so it was green while
    /// `gpui_render_plan_json` was emitting RFC 8259-invalid documents for
    /// every text node containing a newline. That is not hypothetical: a real
    /// `codetracer-gpui --plan-out` artefact taken on 2026-09-22 failed
    /// Python's `json.load` at byte 39045 on the newline inside Python's
    /// `builtins` docstring, while Nim's LENIENT `std/json` parsed the same
    /// bytes without complaint. One document, two parsers, and only the
    /// lenient one had ever been asked.
    ///
    /// `get_plan_json` goes through `serde_json::from_str(...).expect(...)`,
    /// so the strictness is the harness's rather than this assertion's: a
    /// regression fails at the parse, before any `assert_eq!` runs.
    #[test]
    #[serial]
    fn test_render_plan_json_escapes_control_characters() {
        unsafe {
            gpui_reset_tree();
            // Every class the escaper distinguishes, in one string: the two
            // RFC-named ones it already handled, the three with short escapes,
            // and one from the `\u00XX` tail that has no short form.
            let raw = "line1\nline2\tcol\r\u{0b}end \\ \"q\"";
            let node = gpui_create_text_node(c(raw).as_ptr());
            let json = get_plan_json(node);
            // Round trip: the parsed value is the ORIGINAL string, not an
            // escaped rendering of it. An escaper that emitted a literal
            // `\n` two-character sequence would also parse, and would be
            // wrong — which is why this compares against `raw` rather than
            // merely asserting that parsing succeeded.
            assert_eq!(json["text"], raw);
            gpui_destroy_element(node);
        }
    }

    /// **The plan reports the element's public attributes.**
    ///
    /// Written because it did not: the CodeTracer editor records its debugger
    /// surfaces as `data-ct-*` attributes and not one of them appeared in any
    /// captured plan, so the introspection surface could not distinguish a
    /// row carrying an execution pointer from a row without one.
    ///
    /// Three properties, each of which a plausible implementation gets wrong:
    /// a public attribute IS reported; an internal `__` one is NOT (it would
    /// duplicate `text`); and a value carrying a newline round-trips, because
    /// `data-ct-values` holds program values and `get_plan_json` parses with a
    /// strict parser that rejects raw control characters.
    #[test]
    #[serial]
    fn test_render_plan_reports_public_attributes() {
        unsafe {
            gpui_reset_tree();
            let node = gpui_create_element(c("div").as_ptr());
            gpui_set_attribute(node, c("data-ct-pointer").as_ptr(), c("execution").as_ptr());
            gpui_set_attribute(node, c("data-ct-values").as_ptr(), c("x: 1\ny: 2").as_ptr());
            let json = get_plan_json(node);
            assert_eq!(json["attributes"]["data-ct-pointer"], "execution");
            assert_eq!(json["attributes"]["data-ct-values"], "x: 1\ny: 2");
            assert!(json["attributes"].get("__text_content").is_none());
            gpui_destroy_element(node);
        }
    }

    /// A node with no public attributes reports an EMPTY object, not a
    /// missing key — so a reader can tell "none" from "not serialised", which
    /// is the distinction whose absence hid the four surfaces in the first place.
    #[test]
    #[serial]
    fn test_render_plan_attributes_empty_is_an_object() {
        unsafe {
            gpui_reset_tree();
            let node = gpui_create_element(c("div").as_ptr());
            let json = get_plan_json(node);
            assert!(json["attributes"].is_object());
            assert_eq!(json["attributes"].as_object().unwrap().len(), 0);
            gpui_destroy_element(node);
        }
    }

    /// **The plan does not report `letter-spacing`**, because the window does
    /// not draw it (issue
    /// `2026-10-17-letter-spacing-is-parsed-and-reported-but-never-applied.md`).
    /// A plan that lists a property as set while the pixels ignore it lets a
    /// plan-level assertion pass on a frame that is wrong. `line-height`, set
    /// on the same node, is the positive control: it is drawn, so it is
    /// reported, which shows the `styles` object is being populated at all.
    #[test]
    #[serial]
    fn test_render_plan_does_not_report_letter_spacing() {
        unsafe {
            gpui_reset_tree();
            for key in ["letter-spacing", "letter_spacing"] {
                let node = gpui_create_element(c("span").as_ptr());
                gpui_set_style(node, c(key).as_ptr(), c("2px").as_ptr());
                gpui_set_style(node, c("line-height").as_ptr(), c("20px").as_ptr());
                let json = get_plan_json(node);
                assert_eq!(json["styles"]["line_height"], "20px");
                assert!(
                    json["styles"].get("letter_spacing").is_none(),
                    "`{key}` must not appear in the plan: {}",
                    json["styles"]
                );
                gpui_destroy_element(node);
            }
        }
    }

    #[test]
    #[serial]
    fn test_render_plan_verify() {
        unsafe {
            gpui_reset_tree();
            let div = gpui_create_element(c("div").as_ptr());
            assert_eq!(gpui_verify_render_plan(div), 1);
            gpui_destroy_element(div);
        }
    }

    // ===================================================================
    // G2-G: Event dispatch tests
    // ===================================================================

    /// Multiple listeners on the same event all fire, and fire the correct
    /// number of times across repeated dispatches.
    #[test]
    #[serial]
    fn test_dispatch_multiple_listeners_same_event() {
        unsafe {
            gpui_reset_tree();

            static CALL_A: AtomicU32 = AtomicU32::new(0);
            static CALL_B: AtomicU32 = AtomicU32::new(0);
            static CALL_C: AtomicU32 = AtomicU32::new(0);

            extern "C" fn handler_a(_p: *const crate::input::GpuiEventPayload) {
                CALL_A.fetch_add(1, Ordering::SeqCst);
            }
            extern "C" fn handler_b(_p: *const crate::input::GpuiEventPayload) {
                CALL_B.fetch_add(1, Ordering::SeqCst);
            }
            extern "C" fn handler_c(_p: *const crate::input::GpuiEventPayload) {
                CALL_C.fetch_add(1, Ordering::SeqCst);
            }

            CALL_A.store(0, Ordering::SeqCst);
            CALL_B.store(0, Ordering::SeqCst);
            CALL_C.store(0, Ordering::SeqCst);

            let tag = c("button");
            let click = c("click");
            let node = gpui_create_element(tag.as_ptr());

            gpui_add_event_listener(node, click.as_ptr(), handler_a);
            gpui_add_event_listener(node, click.as_ptr(), handler_b);
            gpui_add_event_listener(node, click.as_ptr(), handler_c);

            // First dispatch: all three handlers fire once
            gpui_dispatch_event(node, click.as_ptr());
            assert_eq!(CALL_A.load(Ordering::SeqCst), 1);
            assert_eq!(CALL_B.load(Ordering::SeqCst), 1);
            assert_eq!(CALL_C.load(Ordering::SeqCst), 1);

            // Second dispatch: counts increment to 2
            gpui_dispatch_event(node, click.as_ptr());
            assert_eq!(CALL_A.load(Ordering::SeqCst), 2);
            assert_eq!(CALL_B.load(Ordering::SeqCst), 2);
            assert_eq!(CALL_C.load(Ordering::SeqCst), 2);

            gpui_destroy_element(node);
        }
    }

    /// Dispatching "click" only fires click handlers; dispatching "input"
    /// only fires input handlers. Events on different nodes are isolated.
    #[test]
    #[serial]
    fn test_dispatch_isolation_click_vs_input_different_nodes() {
        unsafe {
            gpui_reset_tree();

            static CLICK_COUNT: AtomicU32 = AtomicU32::new(0);
            static INPUT_COUNT: AtomicU32 = AtomicU32::new(0);

            extern "C" fn on_click(_p: *const crate::input::GpuiEventPayload) {
                CLICK_COUNT.fetch_add(1, Ordering::SeqCst);
            }
            extern "C" fn on_input(_p: *const crate::input::GpuiEventPayload) {
                INPUT_COUNT.fetch_add(1, Ordering::SeqCst);
            }

            CLICK_COUNT.store(0, Ordering::SeqCst);
            INPUT_COUNT.store(0, Ordering::SeqCst);

            let root = gpui_create_element(c("div").as_ptr());
            let btn = gpui_create_element(c("button").as_ptr());
            let inp = gpui_create_element(c("div").as_ptr());

            gpui_add_event_listener(btn, c("click").as_ptr(), on_click);
            gpui_add_event_listener(inp, c("input").as_ptr(), on_input);

            gpui_append_child(root, btn);
            gpui_append_child(root, inp);

            // Dispatch click on button — only click fires
            gpui_dispatch_event(btn, c("click").as_ptr());
            assert_eq!(CLICK_COUNT.load(Ordering::SeqCst), 1);
            assert_eq!(INPUT_COUNT.load(Ordering::SeqCst), 0);

            // Dispatch input on input node — only input fires
            gpui_dispatch_event(inp, c("input").as_ptr());
            assert_eq!(CLICK_COUNT.load(Ordering::SeqCst), 1);
            assert_eq!(INPUT_COUNT.load(Ordering::SeqCst), 1);

            // Dispatch click on input node — nothing fires (no click listener)
            gpui_dispatch_event(inp, c("click").as_ptr());
            assert_eq!(CLICK_COUNT.load(Ordering::SeqCst), 1);
            assert_eq!(INPUT_COUNT.load(Ordering::SeqCst), 1);

            // Dispatch input on button — nothing fires (no input listener)
            gpui_dispatch_event(btn, c("input").as_ptr());
            assert_eq!(CLICK_COUNT.load(Ordering::SeqCst), 1);
            assert_eq!(INPUT_COUNT.load(Ordering::SeqCst), 1);

            gpui_destroy_element(root);
            gpui_destroy_element(btn);
            gpui_destroy_element(inp);
        }
    }

    /// Dispatching events on a null handle does not crash.
    #[test]
    #[serial]
    fn test_dispatch_null_node_safety() {
        unsafe {
            let event = c("click");
            // Should not crash or panic
            gpui_dispatch_event(std::ptr::null_mut(), event.as_ptr());
        }
    }

    /// Dispatching a non-existent event name on a node with handlers
    /// does not crash and does not fire any handler.
    #[test]
    #[serial]
    fn test_dispatch_nonexistent_event_name() {
        unsafe {
            gpui_reset_tree();

            static CALL: AtomicU32 = AtomicU32::new(0);
            extern "C" fn handler(_p: *const crate::input::GpuiEventPayload) {
                CALL.fetch_add(1, Ordering::SeqCst);
            }
            CALL.store(0, Ordering::SeqCst);

            let node = gpui_create_element(c("div").as_ptr());
            gpui_add_event_listener(node, c("click").as_ptr(), handler);

            // Dispatch an event that has no listeners
            gpui_dispatch_event(node, c("mouseover").as_ptr());
            assert_eq!(CALL.load(Ordering::SeqCst), 0);

            gpui_destroy_element(node);
        }
    }

    /// The tree lock is released before event callbacks are invoked.
    /// A callback that accesses the tree should not deadlock.
    #[test]
    #[serial]
    fn test_lock_release_before_callback_no_deadlock() {
        unsafe {
            gpui_reset_tree();

            static SUCCESS: AtomicU32 = AtomicU32::new(0);

            extern "C" fn tree_accessing_handler(_p: *const crate::input::GpuiEventPayload) {
                // If the lock were still held, this would deadlock.
                let count = crate::gpui_tree_node_count();
                if count > 0 {
                    SUCCESS.store(1, Ordering::SeqCst);
                }
            }

            SUCCESS.store(0, Ordering::SeqCst);

            let node = gpui_create_element(c("div").as_ptr());
            gpui_add_event_listener(node, c("click").as_ptr(), tree_accessing_handler);

            gpui_dispatch_event(node, c("click").as_ptr());
            assert_eq!(
                SUCCESS.load(Ordering::SeqCst),
                1,
                "callback should have accessed tree without deadlock"
            );

            gpui_destroy_element(node);
        }
    }

    /// Also verify dispatch_shadow_event (the render-sync path) releases
    /// the lock before callbacks, just like gpui_dispatch_event.
    #[test]
    #[serial]
    fn test_dispatch_shadow_event_lock_release() {
        gpui_reset_tree();

        static SUCCESS: AtomicU32 = AtomicU32::new(0);

        extern "C" fn tree_reader(_p: *const crate::input::GpuiEventPayload) {
            // Access the tree to verify lock is released
            let count = crate::gpui_tree_node_count();
            if count > 0 {
                SUCCESS.store(1, Ordering::SeqCst);
            }
        }

        SUCCESS.store(0, Ordering::SeqCst);

        let node_id = {
            let mut tree = lock_tree();
            let mut node = Node::new_element("button");
            node.event_listeners
                .entry("click".into())
                .or_default()
                .push(crate::tree::EventListener {
                    callback: tree_reader,
                    callback_id: 0,
                });
            tree.insert(node)
        };

        // Use the render-sync dispatch path (same code path GPUI uses)
        #[cfg(feature = "gpui-backend")]
        {
            crate::render_sync::gpui_render::dispatch_shadow_event(node_id, "click");
            assert_eq!(SUCCESS.load(Ordering::SeqCst), 1);
        }

        // Without gpui-backend, verify via the FFI dispatch
        #[cfg(not(feature = "gpui-backend"))]
        {
            let handle = crate::node_id_to_handle(node_id);
            unsafe {
                gpui_dispatch_event(handle, c("click").as_ptr());
            }
            assert_eq!(
                SUCCESS.load(Ordering::SeqCst),
                1,
                "dispatch_shadow_event should release lock before callback"
            );
            unsafe {
                crate::gpui_destroy_element(handle);
            }
        }
    }

    /// End-to-end: populate tree via gpui_launch, then dispatch events.
    /// Without `gpui-backend`, gpui_launch builds the tree and returns
    /// immediately (no event loop), so we can verify the tree and dispatch.
    #[test]
    #[serial]
    #[cfg(not(feature = "gpui-backend"))]
    fn test_window_integration_with_launch() {
        use crate::{
            gpui_first_child, gpui_launch, gpui_reset_windows, gpui_tree_node_count,
            node_id_to_handle, GpuiElement, ROOT_NODE_ID,
        };

        gpui_reset_tree();
        gpui_reset_windows();

        static CLICK_FIRED: AtomicU32 = AtomicU32::new(0);

        extern "C" fn on_click(_p: *const crate::input::GpuiEventPayload) {
            CLICK_FIRED.fetch_add(1, Ordering::SeqCst);
        }

        extern "C" fn builder(root: *mut GpuiElement) {
            unsafe {
                let tag = CString::new("button").unwrap();
                let click = CString::new("click").unwrap();
                let btn = gpui_create_element(tag.as_ptr());
                gpui_add_event_listener(btn, click.as_ptr(), on_click);
                gpui_append_child(root, btn);

                // Add a second child (span with text) to verify tree structure
                let span_tag = CString::new("span").unwrap();
                let span = gpui_create_element(span_tag.as_ptr());
                let text = crate::gpui_create_text_node(
                    CString::new("hello").unwrap().as_ptr(),
                );
                crate::gpui_append_child(span, text);
                gpui_append_child(root, span);

                gpui_destroy_element(btn);
                gpui_destroy_element(span);
                gpui_destroy_element(text);
            }
        }

        CLICK_FIRED.store(0, Ordering::SeqCst);

        let title = c("Test Window");
        gpui_launch(title.as_ptr(), 800.0, 600.0, builder);

        // Root + button + span + text = 4 nodes
        assert_eq!(gpui_tree_node_count(), 4);

        // Get the root handle and verify structure
        let root_id = {
            let root = ROOT_NODE_ID.lock().unwrap();
            *root
        };
        let root_handle = node_id_to_handle(root_id);

        // Verify render plan
        let plan = get_plan_direct(root_handle);
        assert_eq!(plan.children.len(), 2);
        assert!(plan.children[0].has_click_handler); // button
        assert_eq!(plan.children[1].kind, GpuiElementKind::TextContainer); // span

        // Find the button (first child of root) and dispatch a click
        let btn_handle = unsafe { gpui_first_child(root_handle) };
        assert!(!btn_handle.is_null());

        let click = c("click");
        gpui_dispatch_event(btn_handle, click.as_ptr());
        assert_eq!(CLICK_FIRED.load(Ordering::SeqCst), 1);

        // Dispatch again
        gpui_dispatch_event(btn_handle, click.as_ptr());
        assert_eq!(CLICK_FIRED.load(Ordering::SeqCst), 2);

        unsafe {
            gpui_destroy_element(root_handle);
            gpui_destroy_element(btn_handle);
        }
    }
}
