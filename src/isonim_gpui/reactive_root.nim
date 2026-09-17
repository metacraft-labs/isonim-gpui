## isonim_gpui/reactive_root.nim — NH-M1 reactive root entry point for GPUI.
##
## `renderGpui` is GPUI's half of the native mount seam described in
## `codetracer-specs/Front-Ends/IsoNim/Hot-Module-Reload-Native.milestones.org`
## (NH-M1). It delegates to `isonim/renderers/native`'s `renderNative`, which
## opens a `createRoot` scope and re-runs the accessor inside a
## `createRenderEffect`, so NH-M2's hot-component proxy can replace the root
## element without disposing the reactive root.
##
## GPUI's element tree is owned by the Rust shim and has no ambient "document"
## to attach to: the launcher holds the root handle itself and feeds it to the
## frame source. So the surface step here is a caller-supplied `mount` callback
## (`renderGpui(accessor, mount)`), plus a parent-element overload for the case
## where the root really is inserted under an existing shim element.
##
## SHIM DEPENDENCY. Every proc in `bindings.nim` is `dynlib:
## libgpui_nim_shim.so`, so any binary that reaches the shim needs the Rust
## cdylib present at run time. The accessor/mount overload touches no shim
## symbol of its own — it is the tree the accessor builds that does.

import ./renderer
import ./bindings

import isonim/renderers/native as native_root
export native_root.NativeRootAccessor, native_root.NativeRootMount,
       native_root.NativeRootHandle, native_root.renderNative,
       native_root.staticNativeRoot, native_root.dispose,
       native_root.isDisposed

proc renderGpui*(accessor: NativeRootAccessor[GpuiElement];
                 mount: NativeRootMount[GpuiElement]
                ): NativeRootHandle[GpuiElement] =
  ## Mount `accessor`'s GPUI element tree through a reactive root. `mount`
  ## publishes the current root wherever the host needs it (a frame source, a
  ## window's root builder, a test's capture slot).
  renderNative(accessor, mount)

proc renderGpui*(r: GpuiRenderer; host: GpuiElement;
                 accessor: NativeRootAccessor[GpuiElement]
                ): NativeRootHandle[GpuiElement] =
  ## Overload for the case where the root is inserted under an existing shim
  ## element: the reactive insert goes through `GpuiRenderer`'s own
  ## `appendChild` / `removeChild`, so the renderer's tree-mutation API stays
  ## the single reconciliation primitive.
  renderNative(r, host, accessor)
