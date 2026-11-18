//! Cached GPUI views at subtree boundaries: the half of "render only what
//! changed" that lives below the render plan.
//!
//! `render_sync::plan_incremental` keeps the PLAN proportional to what
//! changed. GPUI is declarative, though: whatever element tree `render`
//! returns is laid out, prepainted and painted again, so a plan that was
//! cheap to produce would still cost a full element construction, a full
//! Taffy layout and a full paint every frame. A cached view is GPUI's way
//! out (`gpui-pre` `view.rs`, `Entity::cached`): an entity embedded with a
//! layout style, whose element subtree, prepaint and paint GPUI replays from
//! the previous frame for as long as the entity is not invalidated and its
//! bounds, content mask and text style are the same.
//!
//! ## When a cached view is invalidated
//!
//! By the shadow tree's dirty marks, so no second dependency graph is built
//! beside IsoNim's. Every plan node carries its subtree's change stamp
//! (`RenderNode::stamp`); a boundary remembers the stamp it was drawn at,
//! and a different stamp means something in the subtree changed. The
//! boundary then gets a NEW entity. A new entity has a new element id, so
//! GPUI has nothing to replay and renders it. (Notifying the old entity
//! would not do: the frame's dirty-view set is fixed when the draw starts,
//! and the changes this frame shows can be made during it, by the resize and
//! window-state callbacks `NimRootView::render` runs.) A subtree's entity
//! carries no state of its own beyond the boundaries below it, so replacing
//! it loses nothing; the shim keeps no GPUI element state (no `.id()`, no
//! GPUI-managed scrolling) under a boundary.
//!
//! The two read-back stores the shim records during prepaint
//! (`layout_bounds`, `cut_text`) are written only when an element is
//! prepainted, which a replayed subtree is not. Their values are still
//! right after a replay, because a replay happens only at the same bounds,
//! but a host that RESETS a store expects the next frame to fill it again;
//! each reset therefore bumps `frame_stats::cached_subtree_epoch`, and a
//! boundary drawn at an older epoch is drawn again.
//!
//! ## Which subtrees are boundaries: the policy
//!
//! A cached view is laid out from its style alone, never measured from its
//! contents (`Entity::cached`'s documentation: "caching requires a definite
//! size"). So a subtree can be cached only where its box does not depend on
//! what is inside it; anywhere else, caching would change the layout. The
//! rule (`definite_box`) is written against the GPUI style the element
//! would be drawn with, and it is deliberately conservative:
//!
//! * A width or height in px is definite; a percentage is definite when the
//!   parent's matching size is.
//! * In a BLOCK parent an auto width fills the parent (definite when the
//!   parent's is); an auto height is the content's, so never definite.
//! * In a FLEX parent, the main size is definite when the flex basis (or,
//!   for an auto basis, the main size) is, the container does not wrap, and
//!   no content-based minimum can stop the item shrinking (it does not
//!   shrink, states a minimum, or clips on that axis); the cross size is
//!   definite when stated, or when the item stretches across a parent whose
//!   cross size is definite.
//! * An absolutely positioned box needs both sizes stated.
//! * Grid parents, `display: none` and an aspect ratio are never cached.
//! * The boundary must itself be a FLEX container. A block box would let
//!   its children's margins collapse through its edges into its parent's
//!   layout, which a leaf laid out from the style alone cannot reproduce.
//!
//! On top of that, a subtree must hold at least `boundary_min_nodes()` plan
//! nodes: below that, the bookkeeping of a cached view costs more than
//! building the elements it saves. The threshold is MEASURED, not guessed:
//! `tests/frame_work_probe.rs` times frames of a list whose rows hold k
//! nodes, cached and uncached, and `docs/incremental-rendering.md` records
//! the measurement the default was chosen from.
//!
//! Boundaries are placed at the DEEPEST eligible level along each path: a
//! subtree with an eligible child is built as plain elements, and its
//! children become the boundaries. GPUI renders everything below a cached
//! view it draws again as new (it sets `refreshing` for the view's whole
//! subtree), so a list behind a cached view would redraw every row whenever
//! one row changed; drawn plain, it rebuilds one cheap element per row and
//! replays the rows that did not change.
//!
//! Inside a boundary, the subtree's root is laid out as a GPUI layout root at
//! the bounds its parent gave the cached view: its own size becomes 100% of
//! those bounds, and the properties that placed it in its parent (margin,
//! position and inset, minimum and maximum sizes, flex item properties) are
//! dropped, because the parent already applied them to the cached view.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpui::{
    div, relative, AnyElement, AnyView, App, AppContext as _, Context, Display, Entity,
    FlexDirection, FlexWrap, IntoElement, Length, Overflow, Position, Refineable, Render, Style,
    StyleRefinement, Styled, Window,
};

use crate::render_sync::RenderNode;
use crate::tree::{GpuiElementKind, NodeId};

/// The smallest subtree, in plan nodes, that is drawn behind a cached view.
/// See the module documentation and `docs/incremental-rendering.md` for how
/// the default was measured.
pub const DEFAULT_BOUNDARY_MIN_NODES: usize = 2;

static BOUNDARY_MIN_NODES: AtomicUsize = AtomicUsize::new(DEFAULT_BOUNDARY_MIN_NODES);

/// The current threshold.
pub fn boundary_min_nodes() -> usize {
    BOUNDARY_MIN_NODES.load(Ordering::Relaxed)
}

/// Change the threshold; `usize::MAX` turns cached views off. For the
/// measurements and the equivalence tests, which draw one tree both ways.
pub fn set_boundary_min_nodes(n: usize) {
    BOUNDARY_MIN_NODES.store(n, Ordering::Relaxed);
}

/// How an element's parent lays it out, and whether the element's own box
/// is definite, i.e. independent of its contents. Computed top-down while
/// the elements are built, from the GPUI styles they are built with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutBox {
    pub layout: ChildLayout,
    /// Children wrap onto several lines (a flex container only).
    pub wraps: bool,
    /// Children stretch across the cross axis unless they say otherwise.
    pub stretches: bool,
    pub definite_w: bool,
    pub definite_h: bool,
}

/// The algorithm an element lays its children out with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildLayout {
    Block,
    FlexRow,
    FlexColumn,
    /// Grid, or `display: none`: nothing below it is cached.
    Other,
}

impl LayoutBox {
    /// The box the root of the plan is placed in: the window's content,
    /// whose size is known.
    pub const WINDOW: LayoutBox = LayoutBox {
        layout: ChildLayout::Block,
        wraps: false,
        stretches: false,
        definite_w: true,
        definite_h: true,
    };

    /// The box of an element with `style`, placed in `parent`.
    pub fn of(style: &StyleRefinement, parent: &LayoutBox) -> LayoutBox {
        let s = resolve(style);
        let (definite_w, definite_h) = definite_box(&s, parent);
        LayoutBox::with_sizes(&s, definite_w, definite_h)
    }

    /// The box of the root of a cached view, which is laid out at the
    /// definite bounds its parent gave the view.
    pub fn view_root(style: &StyleRefinement) -> LayoutBox {
        LayoutBox::with_sizes(&resolve(style), true, true)
    }

    fn with_sizes(s: &Style, definite_w: bool, definite_h: bool) -> LayoutBox {
        let layout = match s.display {
            Display::Block => ChildLayout::Block,
            Display::Flex => match s.flex_direction {
                FlexDirection::Row | FlexDirection::RowReverse => ChildLayout::FlexRow,
                FlexDirection::Column | FlexDirection::ColumnReverse => ChildLayout::FlexColumn,
            },
            _ => ChildLayout::Other,
        };
        LayoutBox {
            layout,
            wraps: s.flex_wrap != FlexWrap::NoWrap,
            stretches: matches!(s.align_items, None | Some(gpui::AlignItems::Stretch)),
            definite_w,
            definite_h,
        }
    }
}

fn resolve(refinement: &StyleRefinement) -> Style {
    let mut s = Style::default();
    s.refine(refinement);
    s
}

/// Whether a length is definite: `Some(true)` for px or rems, the parent's
/// answer for a percentage, `None` for auto (the caller decides).
fn stated(len: Length, parent_definite: bool) -> Option<bool> {
    match len {
        Length::Definite(gpui::DefiniteLength::Absolute(_)) => Some(true),
        Length::Definite(gpui::DefiniteLength::Fraction(_)) => Some(parent_definite),
        Length::Auto => None,
    }
}

/// Whether the box of an element with style `s`, placed in `parent`, is
/// independent of its contents, per axis. The module documentation states
/// the rule; every case not covered answers "no".
pub fn definite_box(s: &Style, parent: &LayoutBox) -> (bool, bool) {
    if s.display == Display::None || s.aspect_ratio.is_some() {
        return (false, false);
    }
    let w = stated(s.size.width, parent.definite_w);
    let h = stated(s.size.height, parent.definite_h);
    if s.position == Position::Absolute {
        return (w == Some(true), h == Some(true));
    }
    match parent.layout {
        ChildLayout::Block => (w.unwrap_or(parent.definite_w), h.unwrap_or(false)),
        ChildLayout::FlexRow => (
            flex_main_definite(s, s.size.width, s.min_size.width, s.overflow.x, parent, parent.definite_w),
            flex_cross_definite(s, h, parent.definite_h, parent),
        ),
        ChildLayout::FlexColumn => (
            flex_cross_definite(s, w, parent.definite_w, parent),
            flex_main_definite(s, s.size.height, s.min_size.height, s.overflow.y, parent, parent.definite_h),
        ),
        ChildLayout::Other => (false, false),
    }
}

fn flex_main_definite(
    s: &Style,
    size: Length,
    min: Length,
    overflow: Overflow,
    parent: &LayoutBox,
    parent_definite: bool,
) -> bool {
    if parent.wraps {
        return false;
    }
    let basis = match s.flex_basis {
        Length::Auto => size,
        b => b,
    };
    if stated(basis, parent_definite) != Some(true) {
        return false;
    }
    // A flex item's automatic minimum size is its content's minimum, which
    // a leaf laid out from the style alone does not have: it could shrink
    // further than the real subtree would.
    s.flex_shrink == 0.0 || min != Length::Auto || overflow != Overflow::Visible
}

fn flex_cross_definite(s: &Style, stated_size: Option<bool>, parent_definite: bool, parent: &LayoutBox) -> bool {
    if parent.wraps {
        return false;
    }
    match stated_size {
        Some(d) => d,
        None => {
            let stretches = match s.align_self {
                None => parent.stretches,
                Some(a) => a == gpui::AlignSelf::Stretch,
            };
            stretches && parent_definite
        }
    }
}

/// What a parent remembers about one child it considered for a boundary.
enum Entry {
    /// Drawn behind this view, at this stamp and epoch, in this parent box,
    /// laid out with this style.
    Cached {
        entity: Entity<NimSubtreeView>,
        stamp: u64,
        epoch: u64,
        parent: LayoutBox,
        style: StyleRefinement,
    },
    /// Not a boundary at this stamp and parent box: built as ordinary
    /// elements without being asked again.
    Plain { stamp: u64, parent: LayoutBox },
}

/// The boundaries one view (the root, or a cached subtree) drew below it,
/// keyed by shadow node id. Each frame that view renders, it builds a new
/// set, taking what it can from the previous one; whatever it did not take
/// is dropped with the old set, so a boundary lives exactly as long as the
/// view that draws it keeps drawing it.
#[derive(Default)]
pub struct Boundaries {
    entries: HashMap<u64, Entry>,
}

/// The state element construction carries when it may place boundaries.
pub struct BuildCx<'a> {
    app: &'a mut App,
    old: HashMap<u64, Entry>,
    new: HashMap<u64, Entry>,
}

impl<'a> BuildCx<'a> {
    /// Start building, taking `previous` (the last frame's boundaries).
    pub fn new(app: &'a mut App, previous: &mut Boundaries) -> Self {
        BuildCx {
            app,
            old: std::mem::take(&mut previous.entries),
            new: HashMap::new(),
        }
    }

    /// Finish building: the boundaries this frame drew.
    pub fn finish(self) -> Boundaries {
        Boundaries { entries: self.new }
    }

    /// The cached view `plan` is drawn behind, when it is a boundary in
    /// `parent`; `None` to build it as ordinary elements.
    pub fn boundary(&mut self, plan: &RenderNode, parent: &LayoutBox) -> Option<AnyElement> {
        if plan.subtree_nodes < boundary_min_nodes()
            || !matches!(plan.kind, GpuiElementKind::Div | GpuiElementKind::TextContainer)
        {
            return None;
        }
        let key = plan.node_id;
        let epoch = crate::frame_stats::cached_subtree_epoch();
        // The boundaries below a subtree that changed are handed on to its
        // new view: a changed row in an unchanged list replaces the list's
        // view, and the list's other rows must still find theirs.
        let mut inherited = Boundaries::default();
        match self.old.remove(&key) {
            Some(Entry::Cached { entity, stamp, epoch: e, parent: p, style })
                if stamp == plan.stamp && e == epoch && p == *parent =>
            {
                crate::frame_stats::note_subtree_reused();
                let element = AnyView::from(entity.clone()).cached(style.clone()).into_any_element();
                self.new.insert(key, Entry::Cached { entity, stamp, epoch, parent: p, style });
                return Some(element);
            }
            Some(Entry::Plain { stamp, parent: p }) if stamp == plan.stamp && p == *parent => {
                self.new.insert(key, Entry::Plain { stamp, parent: p });
                return None;
            }
            Some(Entry::Cached { entity, .. }) => {
                inherited = entity.update(self.app, |view, _| std::mem::take(&mut view.boundaries));
            }
            _ => {}
        }
        let style = outer_style(plan);
        // A boundary is placed at the DEEPEST eligible level: a subtree
        // with an eligible child is built as plain elements and its
        // children become the boundaries. GPUI draws everything below a
        // cached view it renders again as new (it sets `refreshing` for the
        // view's whole subtree), so a boundary above other boundaries would
        // throw away every one of them whenever anything under it changed:
        // one changed row would redraw the whole list.
        let eligible_child = || {
            let own = LayoutBox::of(&style, parent);
            plan.children.iter().any(|c| eligible(c, &own))
        };
        if !is_boundary(&style, parent) || eligible_child() {
            self.new.insert(key, Entry::Plain { stamp: plan.stamp, parent: *parent });
            return None;
        }
        let entity = self.app.new(|_| NimSubtreeView { node_id: key, boundaries: inherited });
        let element = AnyView::from(entity.clone()).cached(style.clone()).into_any_element();
        self.new.insert(
            key,
            Entry::Cached { entity, stamp: plan.stamp, epoch, parent: *parent, style },
        );
        Some(element)
    }
}

// The GPUI style each plan node was last translated to, by node id, with the
// subtree stamp it was translated at. A node's style is read every frame its
// parent is built, to decide whether it is a boundary; translating the
// style strings again each time would cost a parse per child per frame.
thread_local! {
    static OUTER_STYLES: std::cell::RefCell<HashMap<u64, (u64, StyleRefinement)>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Entries kept before the style cache is cleared and rebuilt as needed.
const OUTER_STYLES_CAP: usize = 200_000;

/// The GPUI style `plan`'s element is drawn with, as its parent sees it.
fn outer_style(plan: &RenderNode) -> StyleRefinement {
    OUTER_STYLES.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((stamp, style)) = cache.get(&plan.node_id) {
            if *stamp == plan.stamp {
                return style.clone();
            }
        }
        let mut el = crate::gpui_app::apply_styles_to_div(div(), &plan.styles);
        let style = el.style().clone();
        if cache.len() >= OUTER_STYLES_CAP {
            cache.clear();
        }
        cache.insert(plan.node_id, (plan.stamp, style.clone()));
        style
    })
}

/// Whether `plan` could be drawn behind a cached view in `parent`, by the
/// size threshold, its kind and `is_boundary`.
fn eligible(plan: &RenderNode, parent: &LayoutBox) -> bool {
    plan.subtree_nodes >= boundary_min_nodes()
        && matches!(plan.kind, GpuiElementKind::Div | GpuiElementKind::TextContainer)
        && is_boundary(&outer_style(plan), parent)
}

/// Whether an element with `style` in `parent` may be drawn behind a cached
/// view (see the module documentation).
pub fn is_boundary(style: &StyleRefinement, parent: &LayoutBox) -> bool {
    let s = resolve(style);
    if s.display != Display::Flex {
        return false;
    }
    let (w, h) = definite_box(&s, parent);
    w && h
}

/// Lay a boundary's root out as a layout root at its view's bounds: what
/// placed it in its parent was applied to the cached view already.
fn as_view_root(style: &mut StyleRefinement) {
    style.size.width = Some(relative(1.).into());
    style.size.height = Some(relative(1.).into());
    style.min_size = Default::default();
    style.max_size = Default::default();
    style.margin = Default::default();
    style.inset = Default::default();
    style.position = None;
    style.aspect_ratio = None;
    style.flex_grow = None;
    style.flex_shrink = None;
    style.flex_basis = None;
    style.align_self = None;
}

/// The view a boundary's subtree is drawn by. It holds no state of its own
/// beyond the boundaries below it; the subtree's plan is read from the
/// shadow tree's plan cache, which the root's render brought up to date in
/// this frame.
pub struct NimSubtreeView {
    node_id: u64,
    boundaries: Boundaries,
}

impl Render for NimSubtreeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let started = std::time::Instant::now();
        crate::frame_stats::note_subtree_rendered();
        let plan = crate::lock_tree().cached_plan(NodeId(self.node_id));
        let element = match plan {
            Some(plan) => {
                let mut bcx = BuildCx::new(cx, &mut self.boundaries);
                let mut el = crate::gpui_app::build_plan_div_in(&plan, Some(&mut bcx), None);
                self.boundaries = bcx.finish();
                as_view_root(el.style());
                el.into_any_element()
            }
            None => div().into_any_element(),
        };
        crate::frame_stats::add_render_ns(started.elapsed().as_nanos() as u64);
        element
    }
}
