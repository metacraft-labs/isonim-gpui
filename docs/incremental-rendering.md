# Incremental rendering: per-frame work proportional to what changed

IsoNim's effects change one node at a time. Until this change the shim threw
that precision away at the FFI boundary: on every frame `NimRootView::render`
walked the whole shadow tree to rebuild the render plan
(`render_sync::build_render_plan`) and then built a GPUI element for every plan
node, so one changed label cost a walk, an element construction, a layout and a
paint over every element in the window.

The work is now cut at three places.

1. **Dirty marks in the shadow tree** (`tree.rs`, `ChangeMarks`). Every
   mutation reaches the tree through a `Tree` method: the renderer ops, the
   attribute, style, text, class and listener setters, and `get_mut`, which
   marks the node it hands out. A change marks its node `plan_stale` and walks
   to the root, queueing each node in its parent's `stale_children` and giving
   every ancestor the change's stamp (`subtree_stamp`). State the plan does not
   read (the delivery record of `input.rs`) is written through
   `get_mut_untracked`.
2. **Per-node cached plans** (`render_sync::plan_incremental`). Each node keeps
   the plan last built for it. A clean node answers its cached plan without
   being read; a changed node rebuilds its own part over its children's cached
   plans; a node only above a change keeps its own part and has the changed
   children's plans spliced in. Plan children are `Arc`s, so unchanged subtrees
   are shared, never copied. A frame with no change does no plan work at all.
   The reference walk `build_render_plan` is kept; the incremental plan is
   tested equal to it over random histories of the renderer ops. The style
   translation (`GpuiStyles`), the inline-run fusion (`RenderNode::fused`) and
   the subtree aggregates (`subtree_nodes`, `drag_regions`, the latter replacing
   a whole-tree walk per frame) are all part of the cached plan.
3. **Cached GPUI views at subtree boundaries** (`cached_views.rs`). A subtree
   whose box does not depend on its contents, and which holds at least
   `boundary_min_nodes()` plan nodes, is drawn behind a GPUI cached view, which
   GPUI replays (elements, layout, prepaint and paint) from the previous frame.
   The view is invalidated by the dirty marks: a boundary drawn at an older
   `stamp` gets a new entity, which inherits the boundaries below the old
   one. Boundaries sit at the deepest eligible level of each path, because
   GPUI draws everything below a cached view it renders again as new: a list
   behind one view would redraw all its rows when one changed, while a list
   drawn plain rebuilds one cheap element per row and replays the unchanged
   rows. The module documentation states the boundary rule and why each part
   of it is needed.

`frame_stats` reports, per frame, the shadow nodes the plan build visited, the
plans it rebuilt, the GPUI elements built, and the cached subtrees reused and
rendered (`gpui_frame_work`).

## The probe

`rust/gpui-nim-shim/tests/frame_work_probe.rs` changes one label in a
3,000-row list and asserts that the frame visits, rebuilds and builds no more
than the label's depth plus a constant, that the counts are the same for 300
rows, and that an idle frame visits and rebuilds nothing. Its negative control,
`tools/frame-work-probe-control.sh`, runs the same file against the shim before
this change with only the counters added; there the counts scale with the rows
and the probe fails.

## Behaviour that must not change, and how it is held

* **Layout.** A cached view is laid out from its style alone, so only boxes
  that are definite in their parent are boundaries. `tests/cached_views_equivalence.rs`
  draws a scene of the shapes the shim draws (fixed and percentage sizes, block
  and flex parents, shrinking and fixed rows, margins, borders, an absolute
  overlay, inline runs, cut text) with every eligible subtree cached and with
  none, through a series of changes, and compares every element's laid-out
  bounds and every cut text.
* **Read-back stores.** `layout_bounds` and `cut_text` are written during
  prepaint, which a replayed subtree skips; a reset of either bumps
  `frame_stats::cached_subtree_epoch`, which draws every boundary again.
* **Hit testing and listeners** are replayed with the subtree; the equivalence
  suite clicks a replayed row.
* **Focus and keys** live on the root element, which is never cached.
* **Multiple windows and the headless capture** each have their own boundary
  set; the plan cache is shared and compared by stamp, never by a flag one view
  could clear for another.

## Measurements

See the "Status" section of the PLAT-76 milestone in codetracer-pm
(`milestones/CodeTracer-Platform.milestones.org`), which records the probe's
counts and frame times and the boundary-threshold measurement at the commits
they were measured on.
