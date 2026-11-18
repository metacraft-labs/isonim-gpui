//! Cut text: what an element that declares `text-overflow` draws, and what
//! its cut drops before the ellipsis.
//!
//! # Why the shim draws this text itself
//!
//! `gpui-pre` 0.3.5 cuts a one-line text in its text element
//! (`TextLayout::layout` in `src/elements/text.rs`, with
//! `LineWrapper::truncate_line` in `src/text_system/line_wrapper.rs`). The
//! cut it makes drops whitespace and ASCII punctuation from the end of what
//! it keeps, and nothing else, so a text that lists its parts with a
//! separator outside ASCII -- `a · b`, `a — b` -- is cut to `a ·…` when the
//! box ends just past the separator. A browser drops nothing at all
//! (CSS Overflow 3, <https://www.w3.org/TR/css-overflow-3/#text-overflow>),
//! so a front-end that wants such a cut to read `a…` has to cut in code, and
//! where the cut falls is known only once the text is laid out.
//!
//! So a text node of an element that declares `text-overflow` is drawn by
//! [`CutText`], which lays the text out exactly as `gpui-pre`'s text element
//! does -- the same measured layout, the same "the shaped text fits, so it is
//! not cut" test, the same `truncate_line` call, the same size rounding --
//! and then drops the characters the element names in
//! [`ELLIPSIS_TRIM_ATTRIBUTE`] (and whitespace) from the end of what the cut
//! kept ([`trim_before_affix`]). An element that names none is cut exactly as
//! `gpui-pre` cuts it.
//!
//! # What the window drew, read back
//!
//! A cut is decided during layout, after the render plan was handed over,
//! so the plan cannot say what a text reads on screen. Every [`CutText`]
//! records the text it drew -- whole or cut -- under the node id of the
//! element that declared `text-overflow`, and `gpui_drawn_text` reads it back
//! (Nim: `renderer.drawnText`), as `gpui_layout_bounds` reads back where an
//! element was laid out. The store is feature-less, like `layout_bounds`, so
//! the exported symbol set does not depend on the features.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::{handle_to_node_id, GpuiElement};

/// The attribute an element that declares `text-overflow` names the
/// characters with that its cut drops from the end of what it keeps, besides
/// whitespace: each character of the value is one (`data-ellipsis-trim="·—"`
/// drops a trailing `·` or `—`, and the spaces around it). Absent or empty,
/// the cut is `gpui-pre`'s own.
pub const ELLIPSIS_TRIM_ATTRIBUTE: &str = "data-ellipsis-trim";

/// The attribute an element that declares `text-overflow` names where its
/// cut falls with: `middle` keeps the text's start and end and drops its
/// middle (see [`ELLIPSIS_SEGMENTS_ATTRIBUTE`]); absent, or any other value,
/// the cut falls at the end.
pub const ELLIPSIS_AT_ATTRIBUTE: &str = "data-ellipsis-at";

/// The attribute a middle cut ([`ELLIPSIS_AT_ATTRIBUTE`] `middle`) names the
/// characters that separate the text's segments with, each character one
/// separator (`data-ellipsis-segments="/"` for a path). Given, the cut keeps
/// whole segments ([`middle_cut_candidates`]); absent or empty, the cut is
/// `gpui-pre`'s own middle cut, character by character.
pub const ELLIPSIS_SEGMENTS_ATTRIBUTE: &str = "data-ellipsis-segments";

/// Where an element's cut falls, from its attributes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CutPlace {
    /// The cut drops the text's middle rather than its end.
    pub middle: bool,
    /// For a middle cut, the characters between the segments it keeps whole.
    pub segments: Vec<char>,
}

/// An element's [`CutPlace`], from its attributes.
pub fn cut_place_of(attributes: &std::collections::BTreeMap<String, String>) -> CutPlace {
    let middle = attributes
        .get(ELLIPSIS_AT_ATTRIBUTE)
        .is_some_and(|v| v.trim() == "middle");
    let segments = if middle {
        attributes
            .get(ELLIPSIS_SEGMENTS_ATTRIBUTE)
            .map(|v| v.chars().filter(|c| !c.is_whitespace()).collect())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    CutPlace { middle, segments }
}

/// `head`, `affix` and `tail` joined, with whitespace and the characters of
/// `trim` dropped from the end of `head` and from the start of `tail`: the
/// same rule as [`trim_before_affix`], on both sides of a middle cut.
pub fn join_middle_cut(head: &str, affix: &str, tail: &str, trim: &[char]) -> String {
    let drop = |c: char| c.is_whitespace() || trim.contains(&c);
    format!("{}{affix}{}", head.trim_end_matches(drop), tail.trim_start_matches(drop))
}

/// The middle cuts of a text whose segments `separators` divide, longest
/// first: the text keeps its first segment and its last, and the segments
/// between give way one at a time from the second (`a/b/c/d` gives
/// `a/…/c/d`, then `a/…/d`); then only the last segment is kept (`…/d`).
/// A separator at the very start belongs to the first segment. A text with
/// no segment between its first and its last has only the last candidate,
/// and a text of one segment has none. Each candidate is
/// [`join_middle_cut`] of its two sides, so `trim` applies at the cut.
pub fn middle_cut_candidates(
    text: &str,
    affix: &str,
    separators: &[char],
    trim: &[char],
) -> Vec<String> {
    let is_sep = |c: char| separators.contains(&c);
    // Byte offsets of every separator, a leading one aside.
    let start = text.chars().next().filter(|c| is_sep(*c)).map_or(0, char::len_utf8);
    let seps: Vec<usize> = text
        .char_indices()
        .filter(|(i, c)| *i >= start && is_sep(*c))
        .map(|(i, _)| i)
        .collect();
    let Some(&last) = seps.last() else {
        return Vec::new();
    };
    if last + text[last..].chars().next().map_or(0, char::len_utf8) >= text.len() {
        return Vec::new();
    }
    let first = seps[0];
    let head_end = first + text[first..].chars().next().map_or(0, char::len_utf8);
    let head = &text[..head_end];
    let mut out: Vec<String> = seps[1..]
        .iter()
        .map(|&k| join_middle_cut(head, affix, &text[k..], trim))
        .collect();
    out.push(join_middle_cut("", affix, &text[last..], trim));
    out
}

/// The characters an element's cut drops, from its attributes.
pub fn trim_chars_of(attributes: &std::collections::BTreeMap<String, String>) -> Vec<char> {
    attributes
        .get(ELLIPSIS_TRIM_ATTRIBUTE)
        .map(|v| v.chars().filter(|c| !c.is_whitespace()).collect())
        .unwrap_or_default()
}

/// `cut` -- what the cut kept, followed by `affix` -- without whitespace or a
/// character of `trim` at the end of what it kept, dropped until neither
/// ends it. A text that is not a cut (it does not end in `affix`), or a
/// `trim` with nothing in it, is returned as it is: whitespace alone was
/// already dropped by `gpui-pre`'s cut.
pub fn trim_before_affix(cut: &str, affix: &str, trim: &[char]) -> String {
    if trim.is_empty() || affix.is_empty() || !cut.ends_with(affix) {
        return cut.to_string();
    }
    let mut kept = &cut[..cut.len() - affix.len()];
    loop {
        let shorter = kept.trim_end_matches(|c: char| c.is_whitespace() || trim.contains(&c));
        if shorter.len() == kept.len() {
            break;
        }
        kept = shorter;
    }
    format!("{kept}{affix}")
}

// ---------------------------------------------------------------------------
// The drawn texts, read back
// ---------------------------------------------------------------------------

static DRAWN: Mutex<Option<HashMap<u64, String>>> = Mutex::new(None);

/// Record what the element `node_id` drew in the latest frame.
pub fn record(node_id: u64, text: &str) {
    let mut store = DRAWN.lock().unwrap_or_else(|p| p.into_inner());
    store.get_or_insert_with(HashMap::new).insert(node_id, text.to_string());
}

/// What the element `node_id` drew in the latest frame that laid it out.
pub fn drawn_text_of(node_id: u64) -> Option<String> {
    let store = DRAWN.lock().unwrap_or_else(|p| p.into_inner());
    store.as_ref().and_then(|m| m.get(&node_id).cloned())
}

/// Forget every recorded text.
pub fn reset() {
    let mut store = DRAWN.lock().unwrap_or_else(|p| p.into_inner());
    *store = None;
    // The next frame must prepaint every element again to fill the store.
    crate::frame_stats::invalidate_cached_subtrees();
}

/// What the element `node` -- one that declares `text-overflow` -- drew in
/// the latest frame, its text whole or cut, as UTF-8. Answers the length
/// the text needs (without a terminating NUL) and copies as much of it as
/// fits in `buf` (`buf_len` bytes, NUL-terminated when there is room), as
/// `gpui_get_text_content` does; 0 for an element no frame has drawn such a
/// text for.
#[no_mangle]
pub extern "C" fn gpui_drawn_text(node: *mut GpuiElement, buf: *mut u8, buf_len: u64) -> u64 {
    let id = unsafe { handle_to_node_id(node) };
    if id.is_null() {
        return 0;
    }
    let Some(text) = drawn_text_of(id.0) else {
        return 0;
    };
    let bytes = text.as_bytes();
    if !buf.is_null() && buf_len > 0 {
        let n = bytes.len().min(buf_len as usize - 1);
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, n);
            *buf.add(n) = 0;
        }
    }
    bytes.len() as u64
}

/// Forget every drawn text `gpui_drawn_text` would answer.
#[no_mangle]
pub extern "C" fn gpui_drawn_text_reset() {
    reset();
}

// ---------------------------------------------------------------------------
// The element
// ---------------------------------------------------------------------------

#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
pub use element::{cut_line_end, cut_line_middle, CutText};

#[cfg(any(feature = "gpui-backend", feature = "gpui-headless"))]
mod element {
    use std::borrow::Cow;
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{
        App, AvailableSpace, Bounds, IntoElement, Pixels, SharedString, Size, TextOverflow,
        TruncateFrom, WhiteSpace, Window, WrappedLine,
    };

    /// A text node drawn under an element that declares `text-overflow`:
    /// `gpui-pre`'s text element, line for line, but for the characters it
    /// drops before the ellipsis (`trim`; see the module docs), and it
    /// records what it drew under `owner`, the declaring element's node id.
    pub struct CutText {
        text: SharedString,
        trim: Rc<[char]>,
        owner: u64,
        place: Rc<super::CutPlace>,
    }

    impl CutText {
        pub fn new(text: impl Into<SharedString>, trim: Vec<char>, owner: u64) -> Self {
            Self {
                text: text.into(),
                trim: trim.into(),
                owner,
                place: Rc::new(super::CutPlace::default()),
            }
        }

        /// Where the cut falls (`data-ellipsis-at`, `data-ellipsis-segments`):
        /// at the end unless `place` says the middle.
        pub fn at(mut self, place: super::CutPlace) -> Self {
            self.place = Rc::new(place);
            self
        }
    }

    /// What the measured layout settled on, kept for `paint`.
    #[derive(Default)]
    struct Measured {
        shown: SharedString,
        lines: Vec<WrappedLine>,
        line_height: Pixels,
        wrap_width: Option<Pixels>,
        truncate_width: Option<Pixels>,
        size: Option<Size<Pixels>>,
    }

    #[derive(Default, Clone)]
    pub struct CutTextLayout(Rc<RefCell<Measured>>);

    impl gpui::Element for CutText {
        type RequestLayoutState = CutTextLayout;
        type PrepaintState = ();

        fn id(&self) -> Option<gpui::ElementId> {
            None
        }

        fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
            None
        }

        /// `TextLayout::layout` (gpui-pre 0.3.5, `src/elements/text.rs`) for a
        /// text without highlights, with the trim applied to its cut.
        fn request_layout(
            &mut self,
            _id: Option<&gpui::GlobalElementId>,
            _inspector_id: Option<&gpui::InspectorElementId>,
            window: &mut Window,
            _cx: &mut App,
        ) -> (gpui::LayoutId, Self::RequestLayoutState) {
            let state = CutTextLayout::default();
            let text_style = window.text_style();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            let line_height = window.pixel_snap(
                text_style
                    .line_height
                    .to_pixels(font_size.into(), window.rem_size()),
            );
            let runs = vec![text_style.to_run(self.text.len())];
            let text = self.text.clone();
            let trim = self.trim.clone();
            let place = self.place.clone();
            let measured = state.clone();
            let layout_id = window.request_measured_layout(
                Default::default(),
                move |known_dimensions, available_space, window, cx| {
                    let wrap_width = if text_style.white_space == WhiteSpace::Normal {
                        known_dimensions.width.or(match available_space.width {
                            AvailableSpace::Definite(x) => Some(x),
                            _ => None,
                        })
                    } else {
                        None
                    };
                    let (truncate_width, affix, truncate_from) =
                        if let Some(overflow) = text_style.text_overflow.clone() {
                            let width = known_dimensions.width.or(match available_space.width {
                                AvailableSpace::Definite(x) => match text_style.line_clamp {
                                    Some(max_lines) => Some(x * max_lines),
                                    None => Some(x),
                                },
                                _ => None,
                            });
                            match overflow {
                                TextOverflow::Truncate(s) => (width, s, TruncateFrom::End),
                                TextOverflow::TruncateStart(s) => (width, s, TruncateFrom::Start),
                                TextOverflow::TruncateMiddle(s) => {
                                    (width, s, TruncateFrom::Middle)
                                }
                            }
                        } else {
                            (None, SharedString::from(""), TruncateFrom::End)
                        };

                    // The cache rule of `TextLayout::layout`: an uncut
                    // layout answers a probe at the same wrap width.
                    {
                        let m = measured.0.borrow();
                        if let Some(size) = m.size {
                            if (wrap_width.is_none() || wrap_width == m.wrap_width)
                                && truncate_width.is_none()
                                && m.truncate_width.is_none()
                            {
                                return size;
                            }
                        }
                    }

                    let mut line_wrapper =
                        cx.text_system().line_wrapper(text_style.font(), font_size);
                    let (shown, shown_runs): (SharedString, Cow<'_, [gpui::TextRun]>) =
                        if let Some(truncate_width) = truncate_width {
                            if let (Some(max_lines), Some(wrap_width)) =
                                (text_style.line_clamp, wrap_width)
                            {
                                line_wrapper.truncate_wrapped_line(
                                    text.clone(),
                                    wrap_width,
                                    max_lines,
                                    &affix,
                                    &runs,
                                    truncate_from,
                                )
                            } else if window
                                .text_system()
                                .shape_text(text.clone(), font_size, &runs, None, None)
                                .map(|lines| {
                                    lines
                                        .iter()
                                        .all(|line| line.size(line_height).width <= truncate_width)
                                })
                                .unwrap_or(false)
                            {
                                // Shaped (kerned), the whole text fits: not cut.
                                (text.clone(), Cow::Borrowed(&runs[..]))
                            } else if truncate_from == TruncateFrom::End && place.middle {
                                // A middle cut: each candidate is measured
                                // shaped, by the same test as the whole text.
                                let mut fits = |candidate: &str| {
                                    window
                                        .text_system()
                                        .shape_text(
                                            SharedString::from(candidate.to_string()),
                                            font_size,
                                            &[text_style.to_run(candidate.len())],
                                            None,
                                            None,
                                        )
                                        .map(|lines| {
                                            lines.iter().all(|line| {
                                                line.size(line_height).width <= truncate_width
                                            })
                                        })
                                        .unwrap_or(false)
                                };
                                cut_line_middle(
                                    &mut line_wrapper,
                                    text.clone(),
                                    truncate_width,
                                    &affix,
                                    &runs,
                                    &trim,
                                    &place.segments,
                                    &mut fits,
                                )
                            } else if truncate_from == TruncateFrom::End {
                                cut_line_end(
                                    &mut line_wrapper,
                                    text.clone(),
                                    truncate_width,
                                    &affix,
                                    &runs,
                                    &trim,
                                )
                            } else {
                                line_wrapper.truncate_line(
                                    text.clone(),
                                    truncate_width,
                                    &affix,
                                    &runs,
                                    truncate_from,
                                )
                            }
                        } else {
                            (text.clone(), Cow::Borrowed(&runs[..]))
                        };

                    let lines: Vec<WrappedLine> = window
                        .text_system()
                        .shape_text(shown.clone(), font_size, &shown_runs, wrap_width, text_style.line_clamp)
                        .map(|lines| lines.into_iter().collect())
                        .unwrap_or_default();
                    let mut size: Size<Pixels> = Size::default();
                    for line in &lines {
                        let line_size = line.size(line_height);
                        size.height += line_size.height;
                        size.width = size.width.max(line_size.width).ceil();
                    }
                    *measured.0.borrow_mut() = Measured {
                        shown,
                        lines,
                        line_height,
                        wrap_width,
                        truncate_width,
                        size: Some(size),
                    };
                    size
                },
            );
            (layout_id, state)
        }

        /// Records what the frame draws: the last measure is the one the
        /// layout settled on (an intrinsic-size probe before it measures
        /// the text uncut).
        fn prepaint(
            &mut self,
            _id: Option<&gpui::GlobalElementId>,
            _inspector_id: Option<&gpui::InspectorElementId>,
            _bounds: Bounds<Pixels>,
            layout: &mut Self::RequestLayoutState,
            _window: &mut Window,
            _cx: &mut App,
        ) {
            super::record(self.owner, &layout.0.borrow().shown);
        }

        /// `TextLayout::paint`: each line's background, then its glyphs,
        /// aligned in the bounds, one under the other.
        fn paint(
            &mut self,
            _id: Option<&gpui::GlobalElementId>,
            _inspector_id: Option<&gpui::InspectorElementId>,
            bounds: Bounds<Pixels>,
            layout: &mut Self::RequestLayoutState,
            _prepaint: &mut Self::PrepaintState,
            window: &mut Window,
            cx: &mut App,
        ) {
            let measured = layout.0.borrow();
            let line_height = measured.line_height;
            let align = window.text_style().text_align;
            let mut origin = bounds.origin;
            for line in &measured.lines {
                // A line the atlas cannot take is lost for this frame, as
                // `TextLayout::paint` logs and drops the error.
                let _ = line.paint_background(origin, line_height, align, Some(bounds), window, cx);
                let _ = line.paint(origin, line_height, align, Some(bounds), window, cx);
                origin.y += line.size(line_height).height;
            }
        }
    }

    impl IntoElement for CutText {
        type Element = Self;

        fn into_element(self) -> Self::Element {
            self
        }
    }

    /// A one-line text cut at its end to `width`: `gpui-pre`'s own cut
    /// (`LineWrapper::truncate_line`, which keeps what fits before `affix`
    /// and drops whitespace and ASCII punctuation from its end), then
    /// `trim_before_affix` with `trim`. A text that is not cut is returned
    /// as it is.
    pub fn cut_line_end<'a>(
        wrapper: &mut gpui::LineWrapper,
        text: SharedString,
        width: Pixels,
        affix: &str,
        runs: &'a [gpui::TextRun],
        trim: &[char],
    ) -> (SharedString, Cow<'a, [gpui::TextRun]>) {
        let (shown, shown_runs) =
            wrapper.truncate_line(text.clone(), width, affix, runs, TruncateFrom::End);
        if shown == text || trim.is_empty() {
            return (shown, shown_runs);
        }
        let trimmed = super::trim_before_affix(&shown, affix, trim);
        if trimmed.len() == shown.len() {
            return (shown, shown_runs);
        }
        let runs = runs_for_end_cut(runs, trimmed.len() - affix.len(), affix.len());
        (SharedString::from(trimmed), Cow::Owned(runs))
    }

    /// A one-line text cut in its middle to `width` (an element whose
    /// `data-ellipsis-at` is `middle`). Whole when it `fits`; with
    /// `segments`, otherwise the first of
    /// [`super::middle_cut_candidates`] that `fits` (measured as the window
    /// shapes it), and when none does, the end cut ([`cut_line_end`]);
    /// without, `gpui-pre`'s own middle cut (`LineWrapper::truncate_line`
    /// with `TruncateFrom::Middle`, which keeps about two thirds of the room
    /// before the affix), with the trim applied on both sides of the affix
    /// ([`super::join_middle_cut`]). The text a `CutText` draws is in one
    /// style, so a cut keeps one run.
    #[allow(clippy::too_many_arguments)]
    pub fn cut_line_middle<'a>(
        wrapper: &mut gpui::LineWrapper,
        text: SharedString,
        width: Pixels,
        affix: &str,
        runs: &'a [gpui::TextRun],
        trim: &[char],
        segments: &[char],
        fits: &mut dyn FnMut(&str) -> bool,
    ) -> (SharedString, Cow<'a, [gpui::TextRun]>) {
        if !segments.is_empty() {
            if fits(&text) {
                return (text, Cow::Borrowed(runs));
            }
            for candidate in super::middle_cut_candidates(&text, affix, segments, trim) {
                if fits(&candidate) {
                    let runs = one_run(runs, candidate.len());
                    return (SharedString::from(candidate), Cow::Owned(runs));
                }
            }
            return cut_line_end(wrapper, text, width, affix, runs, trim);
        }
        let (shown, shown_runs) =
            wrapper.truncate_line(text.clone(), width, affix, runs, TruncateFrom::Middle);
        if shown == text || trim.is_empty() || affix.is_empty() {
            return (shown, shown_runs);
        }
        if let Some(at) = shown.find(affix) {
            let joined =
                super::join_middle_cut(&shown[..at], affix, &shown[at + affix.len()..], trim);
            if joined.len() != shown.len() {
                let runs = one_run(runs, joined.len());
                return (SharedString::from(joined), Cow::Owned(runs));
            }
        }
        (shown, shown_runs)
    }

    /// The first run of `runs`, `len` bytes long: the runs of a cut text
    /// drawn in one style.
    fn one_run(runs: &[gpui::TextRun], len: usize) -> Vec<gpui::TextRun> {
        runs.first()
            .map(|run| {
                let mut run = run.clone();
                run.len = len;
                vec![run]
            })
            .unwrap_or_default()
    }

    /// The runs of an end cut that keeps `kept` bytes of the text and adds
    /// an affix of `affix` bytes: `update_runs_after_truncation` in gpui-pre's
    /// `line_wrapper.rs` (private there), for the trimmed cut. The run the
    /// cut falls in also carries the affix; the runs after it go.
    pub(super) fn runs_for_end_cut(
        runs: &[gpui::TextRun],
        kept: usize,
        affix: usize,
    ) -> Vec<gpui::TextRun> {
        let mut runs = runs.to_vec();
        let mut at = kept;
        for i in 0..runs.len() {
            if runs[i].len <= at && i + 1 < runs.len() {
                at -= runs[i].len;
            } else {
                runs[i].len = at + affix;
                runs.truncate(i + 1);
                break;
            }
        }
        runs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOT: char = '\u{b7}';
    const DASH: char = '\u{2014}';

    #[test]
    fn a_cut_drops_a_trailing_separator_and_its_spaces() {
        let trim = [DOT, DASH];
        assert_eq!(trim_before_affix("a \u{b7}\u{2026}", "\u{2026}", &trim), "a\u{2026}");
        assert_eq!(trim_before_affix("a \u{2014}\u{2026}", "\u{2026}", &trim), "a\u{2026}");
        assert_eq!(trim_before_affix("a \u{b7} b \u{b7}\u{2026}", "\u{2026}", &trim), "a \u{b7} b\u{2026}");
        // Repeated: a separator, its space, another separator.
        assert_eq!(trim_before_affix("a\u{2014} \u{b7}\u{2026}", "\u{2026}", &trim), "a\u{2026}");
        // What the cut keeps otherwise stays.
        assert_eq!(trim_before_affix("a.b\u{2026}", "\u{2026}", &trim), "a.b\u{2026}");
        // Nothing left but the affix.
        assert_eq!(trim_before_affix(" \u{b7}\u{2026}", "\u{2026}", &trim), "\u{2026}");
    }

    #[test]
    fn without_a_trim_set_or_a_cut_the_text_is_unchanged() {
        assert_eq!(trim_before_affix("a \u{b7}\u{2026}", "\u{2026}", &[]), "a \u{b7}\u{2026}");
        assert_eq!(trim_before_affix("a \u{b7} b", "\u{2026}", &[DOT]), "a \u{b7} b");
        assert_eq!(trim_before_affix("a \u{b7}", "", &[DOT]), "a \u{b7}");
    }

    #[test]
    fn the_trim_set_is_read_from_the_attribute() {
        let mut attrs = std::collections::BTreeMap::new();
        assert!(trim_chars_of(&attrs).is_empty());
        attrs.insert(ELLIPSIS_TRIM_ATTRIBUTE.to_string(), "\u{b7} \u{2014}".to_string());
        assert_eq!(trim_chars_of(&attrs), vec![DOT, DASH]);
    }

    #[test]
    fn a_middle_cut_keeps_whole_segments_first_and_last() {
        let e = "\u{2026}";
        assert_eq!(
            middle_cut_candidates("src/upload/retry/client.py", e, &['/'], &[]),
            vec![
                "src/\u{2026}/retry/client.py".to_string(),
                "src/\u{2026}/client.py".to_string(),
                "\u{2026}/client.py".to_string()
            ]
        );
        // No segment between the first and the last: only the last kept.
        assert_eq!(middle_cut_candidates("src/client.py", e, &['/'], &[]),
                   vec!["\u{2026}/client.py".to_string()]);
        // One segment, a leading separator, a trailing one: nothing to keep.
        assert!(middle_cut_candidates("client.py", e, &['/'], &[]).is_empty());
        assert!(middle_cut_candidates("/client.py", e, &['/'], &[]).is_empty());
        assert!(middle_cut_candidates("src/lib/", e, &['/'], &[]).is_empty());
        // A leading separator belongs to the first segment.
        assert_eq!(middle_cut_candidates("/srv/a/b.py", e, &['/'], &[])[0],
                   "/srv/\u{2026}/b.py");
    }

    #[test]
    fn a_middle_cut_drops_what_would_dangle_on_either_side() {
        let trim = ['\u{b7}', '\u{2014}'];
        assert_eq!(join_middle_cut("a \u{b7} ", "\u{2026}", " \u{b7} b", &trim), "a\u{2026}b");
        assert_eq!(join_middle_cut("a ", "\u{2026}", " b", &[]), "a\u{2026}b");
        assert_eq!(join_middle_cut("src/", "\u{2026}", "/c.py", &trim), "src/\u{2026}/c.py");
    }

    #[test]
    fn the_cut_place_is_read_from_the_attributes() {
        let mut attrs = std::collections::BTreeMap::new();
        assert_eq!(cut_place_of(&attrs), CutPlace::default());
        attrs.insert(ELLIPSIS_SEGMENTS_ATTRIBUTE.to_string(), "/".to_string());
        // Segments without a middle cut mean nothing.
        assert_eq!(cut_place_of(&attrs), CutPlace::default());
        attrs.insert(ELLIPSIS_AT_ATTRIBUTE.to_string(), "middle".to_string());
        assert_eq!(cut_place_of(&attrs), CutPlace { middle: true, segments: vec!['/'] });
        attrs.insert(ELLIPSIS_AT_ATTRIBUTE.to_string(), "end".to_string());
        assert!(!cut_place_of(&attrs).middle);
    }

    #[test]
    fn records_reads_and_forgets() {
        reset();
        assert_eq!(drawn_text_of(11), None);
        record(11, "a\u{2026}");
        assert_eq!(drawn_text_of(11).as_deref(), Some("a\u{2026}"));
        record(11, "a b");
        assert_eq!(drawn_text_of(11).as_deref(), Some("a b"));
        reset();
        assert_eq!(drawn_text_of(11), None);

        assert_eq!(gpui_drawn_text(std::ptr::null_mut(), std::ptr::null_mut(), 0), 0);
        let el = crate::gpui_create_element(c"div".as_ptr());
        assert_eq!(gpui_drawn_text(el, std::ptr::null_mut(), 0), 0);
        let id = crate::gpui_node_id(el);
        record(id, "ab\u{b7}");
        // The length needed, then the bytes, NUL-terminated.
        assert_eq!(gpui_drawn_text(el, std::ptr::null_mut(), 0), 4);
        let mut buf = [0xffu8; 8];
        assert_eq!(gpui_drawn_text(el, buf.as_mut_ptr(), buf.len() as u64), 4);
        assert_eq!(&buf[..5], "ab\u{b7}\0".as_bytes());
        // A buffer too small takes what fits.
        let mut small = [0xffu8; 3];
        assert_eq!(gpui_drawn_text(el, small.as_mut_ptr(), small.len() as u64), 4);
        assert_eq!(&small, b"ab\0");
        gpui_drawn_text_reset();
        assert_eq!(gpui_drawn_text(el, std::ptr::null_mut(), 0), 0);
        crate::gpui_destroy_element(el);
    }
}
