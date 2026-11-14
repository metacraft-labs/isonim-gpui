## The display session — which platform GPUI will open its window on.
##
## On Linux and FreeBSD GPUI does not ask the application where to draw: it
## reads the process environment once, when the windowed platform is created
## (`gpui_platform::current_platform(false)`, called from the shim's
## `launch_gpui_app`), and picks Wayland, X11 or a HEADLESS platform. The
## headless platform is the trap. It opens no window, has nothing to close,
## and so `gpui_launch` never returns: an application started from an SSH
## session or a bare tty just sits there, silently.
##
## `gpuiLinuxPlatform` is that choice as a pure function of the environment,
## so an application can find out BEFORE calling `gpui_launch` that no window
## will appear, and say so. It mirrors `gpui::guess_compositor` in the crate
## this repository pins (`gpui-pre` =0.3.5, `src/platform.rs`), with the shim's
## `gpui-backend` feature enabling both the `wayland` and the `x11` arm:
##
##   1. `ZED_HEADLESS` present, with ANY value including the empty string
##      (`std::env::var_os(..).is_some()`)          -> headless
##   2. `WAYLAND_DISPLAY` set and non-empty          -> Wayland
##   3. `DISPLAY` set and non-empty                  -> X11
##   4. otherwise                                    -> headless
##
## An empty `WAYLAND_DISPLAY` / `DISPLAY` counts as unset, exactly as GPUI
## counts it. If the pin moves, re-read `guess_compositor` and this together
## (`docs/gpui-pin.md`).
##
## macOS and Windows have no such choice: GPUI talks to AppKit / Win32
## directly, so `gpuiChoosesPlatformFromEnv` is false there and no
## environment check applies.
##
## This module imports nothing from the shim, so it can be used (and tested)
## by a program that has not loaded `libgpui_nim_shim` at all.

import std/[options, os]

type
  GpuiLinuxPlatform* = enum
    ## The platform GPUI's Linux/FreeBSD backend picks for a window.
    glpWayland = "wayland"
    glpX11 = "x11"
    glpHeadless = "headless"
      ## No window can appear: GPUI runs its event loop with no display.

  EnvLookup* = proc(name: string): Option[string]
    ## Reads one environment variable: `none` when it is not set at all,
    ## `some("")` when it is set to the empty string. The distinction matters
    ## for `ZED_HEADLESS`, which GPUI honours by presence alone.

proc processEnv*(name: string): Option[string] =
  ## The `EnvLookup` over this process's real environment.
  if existsEnv(name): some(getEnv(name)) else: none(string)

proc gpuiChoosesPlatformFromEnv*(targetOs: string): bool =
  ## Whether GPUI picks its window platform from the environment on
  ## `targetOs` (spelled as Nim's `hostOS`: "linux", "freebsd", "macosx",
  ## "windows", …). True exactly for the targets `gpui_platform` routes to
  ## `gpui_linux` (`cfg(any(target_os = "linux", target_os = "freebsd"))`).
  targetOs in ["linux", "freebsd"]

proc gpuiLinuxPlatform*(env: EnvLookup): GpuiLinuxPlatform =
  ## The platform GPUI will choose, given the environment `env` reads. See
  ## the module header for the rule and where it comes from.
  if env("ZED_HEADLESS").isSome:
    return glpHeadless
  if env("WAYLAND_DISPLAY").get("").len > 0:
    return glpWayland
  if env("DISPLAY").get("").len > 0:
    return glpX11
  glpHeadless
