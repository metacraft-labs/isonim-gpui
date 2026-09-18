## isonim_gpui/reconciler.nim
##
## NH-M3 — the GPUI instance of IsoNim's ``RendererReconciler`` contract
## (``isonim/native/reconciler``).
##
## ## Difficulty, and where it actually lies
##
## The design doc grades GPUI "medium": element identity has to be
## preserved *through* the existing build-tree path, and re-applied
## styles must not invalidate computed-style caches on unchanged
## elements. This instance addresses that directly — ``updateProps`` is
## called **only when the tracked properties differ**, so an element the
## edit did not touch receives no ``gpui_set_attribute`` /
## ``gpui_set_style`` call at all and its cached computed style stands.
## The engine, not this file, enforces that: see ``reconcileNode``'s
## ``propsDiffer`` guard.
##
## ## What ``properties`` can and cannot see, stated plainly
##
## ``GpuiElement`` is an opaque handle into the Rust shadow tree and the
## shim exports **no enumerate-attributes entry point** — only
## ``gpui_get_attribute(name)``. So ``properties`` returns the values of
## a TRACKED SET of names rather than everything the node carries, and
## ``newTrackedGpuiReconciler`` is how a caller extends it.
##
## That is a real limitation and it is written here rather than
## discovered later: **an attribute outside the tracked set will not be
## updated by a reconcile**, because the diff cannot see that it
## changed. The default set covers what the IsoNim DSL emits today.
## Widening it costs one FFI read per name per node per reconcile; the
## alternative — a new ``gpui_attribute_names`` export — is a shim + Nim
## binding + export-gate change and is the right fix if the tracked set
## ever stops being enumerable.
##
## ## Identity
##
## The key lives in the ``data-isonim-key`` attribute, the same name
## every other instance uses — the const is imported from
## ``isonim/native/reconciler_native`` rather than respelled, so the
## instances cannot drift onto different attribute names. Unkeyed nodes
## fall back to ``defaultIdentityKey(tag, index)``.

when defined(js):
  {.error: "isonim_gpui/reconciler is for native (nim c) targets only.".}

import std/tables
import isonim_gpui/bindings
import isonim_gpui/renderer
import isonim/native/reconciler
import isonim/native/reconciler_native

export reconciler, IsonimKeyAttr

const defaultTrackedProps* = @[
  ## The attribute names a reconcile compares by default. See the module
  ## header for why this is a list rather than "all of them".
  "class", "id", "style", "role", "title", "value", "placeholder",
  "checked", "disabled", "selected", "href", "src", "alt", "type",
  "name", "aria-label", "data-testid",
]

proc gpuiIdentityKey*(n: GpuiElement; index: int): NodeIdentity =
  if n == nil: return ""
  let explicitKey = getAttribute(n, IsonimKeyAttr)
  if explicitKey.len > 0: return explicitKey
  defaultIdentityKey(getTag(n), index)

proc indexInParent(n: GpuiElement): int =
  ## Position of `n` under its parent, for the positional fallback only.
  ## Walks the parent's children through the shim rather than caching,
  ## because the shadow tree is the authority and a cached index would
  ## go stale the moment the reconciler itself moved something.
  if n == nil: return 0
  let r = GpuiRenderer()
  let parent = r.parentNode(n)
  if parent == nil: return 0
  for i in 0 ..< childCount(parent):
    # `sameNode`, NOT `==`. Every child accessor mints a fresh handle,
    # so a pointer comparison here is false for the node we are standing
    # on and this proc would answer 0 for every node in the tree —
    # silently collapsing the positional fallback onto one key.
    if sameNode(nthChild(parent, i), n): return i
  0

proc newGpuiReconciler*(trackedProps: seq[string] = defaultTrackedProps):
    RendererReconciler[GpuiElement] =
  ## The reconciler for ``GpuiRenderer``'s shadow tree.
  ##
  ## Every mutation goes through the RendererBackend wrappers, which go
  ## through the shim — the Rust shadow tree is the single source of
  ## truth for parent/child links, and a Nim-side shortcut would leave
  ## the two views disagreeing.
  let r = GpuiRenderer()
  let tracked = trackedProps
  RendererReconciler[GpuiElement](
    nodes: RendererTreeNodeOps[GpuiElement](
      identityKey: proc(n: GpuiElement): NodeIdentity =
        gpuiIdentityKey(n, indexInParent(n)),
      kind: proc(n: GpuiElement): NodeKind =
        if n == nil: "" else: getTag(n),
      children: proc(n: GpuiElement): seq[GpuiElement] =
        result = @[]
        if n == nil: return
        for i in 0 ..< childCount(n):
          result.add(nthChild(n, i)),
      properties: proc(n: GpuiElement): Table[string, string] =
        result = initTable[string, string]()
        if n == nil: return
        for name in tracked:
          let v = getAttribute(n, name)
          if v.len > 0: result[name] = v
        # TEXT IS A LEAF PROPERTY HERE, and the guard is not an
        # optimisation. ``gpui_get_text_content`` AGGREGATES the text of
        # every descendant, so without it a one-word edit deep in the
        # tree reports a "changed" text property on the edited node AND
        # on every ancestor — measured 2026-09-18 as 3 prop updates for
        # a single edited leaf. Worse, ``updateProps`` would then call
        # ``set_text_content`` on a CONTAINER, which replaces its
        # children: the reconciler would destroy the subtree it exists
        # to preserve.
        if childCount(n) == 0:
          let text = textContent(n)
          if text.len > 0: result["text"] = text),
    placeAt: proc(parent, child: GpuiElement; index: int) =
      if parent == nil or child == nil: return
      if index >= childCount(parent):
        r.appendChild(parent, child)
      else:
        r.insertBefore(parent, child, nthChild(parent, index)),
    move: proc(parent, child: GpuiElement; fromIndex, toIndex: int) =
      if parent == nil or child == nil: return
      r.removeChild(parent, child)
      if toIndex >= childCount(parent):
        r.appendChild(parent, child)
      else:
        r.insertBefore(parent, child, nthChild(parent, toIndex)),
    remove: proc(parent, child: GpuiElement) =
      if parent == nil or child == nil: return
      r.removeChild(parent, child),
    updateProps: proc(node: GpuiElement;
                      oldProps, newProps: Table[string, string]) =
      # Reached only when the engine has already found a difference, so
      # an unchanged element's computed-style cache is never disturbed.
      if node == nil: return
      for k, v in newProps:
        if k == "text":
          # Guarded a second time, at the write. `properties` only
          # produces "text" for a leaf, but a caller may supply its own
          # `properties`, and a `set_text_content` on a container
          # replaces its children.
          if childCount(node) == 0 and textContent(node) != v:
            r.setTextContent(node, v)
        else:
          r.setAttribute(node, k, v)
      for k in oldProps.keys:
        if not newProps.hasKey(k):
          if k == "text":
            if childCount(node) == 0: r.setTextContent(node, "")
          else:
            r.removeAttribute(node, k))
