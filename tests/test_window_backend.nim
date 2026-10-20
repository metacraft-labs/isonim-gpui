## Can a window open at all? — the shim's own answer, and GPUI's platform
## choice.
##
## Build & run (the default, FEATURELESS shim — what `ci/run-suite.sh` and
## `just rust-build` produce):
##   LD_LIBRARY_PATH=rust/target/debug nim c -r tests/test_window_backend.nim
##
## Against a WINDOWED shim, compile with `-d:gpuiBackend` (the define every
## windowed suite in this repo already uses) and point the binary at that
## shim, e.g. `-d:gpuiShimPath=rust/target/windowed/debug/libgpui_nim_shim.so`;
## the expected answer flips with the define.
##
## WHAT IS COVERED
##
##   * `gpui_has_window_backend` across the C boundary, from the shim this
##     binary actually loaded: a featureless build answers 0, which is the
##     answer an application needs in order to refuse instead of returning
##     silently from a `gpui_launch` that opened nothing.
##     The Rust unit test `test_has_window_backend_matches_the_build` holds
##     the same claim inside the crate, for both builds.
##   * `display_session.gpuiLinuxPlatform`, the mirror of
##     `gpui::guess_compositor`: each platform, Wayland's precedence over X11,
##     empty values counted as unset, and `ZED_HEADLESS` winning by presence
##     alone; and `gpuiChoosesPlatformFromEnv` per target.
##
## MOCKS: none in the sense the guidelines mean. The environment is passed as
## a table-backed `EnvLookup` rather than by mutating this process's
## environment, because the function under test is pure by design and the
## cases (an empty `ZED_HEADLESS`, both displays at once) are exactly the
## ones a real session cannot be arranged to show on demand. `processEnv`,
## the production lookup, is exercised against a variable this test sets.

import std/[options, os, tables, unittest]
import isonim_gpui/[bindings, display_session]

proc envOf(pairs: openArray[(string, string)]): EnvLookup =
  ## An `EnvLookup` over exactly `pairs`; a name not listed is unset.
  let t = pairs.toTable
  result = proc(name: string): Option[string] =
    if name in t: some(t[name]) else: none(string)

suite "the shim says whether it can open a window":

  test "gpui_has_window_backend matches the build this binary loaded":
    when defined(gpuiBackend):
      check gpui_has_window_backend() == 1'u8
    else:
      # The featureless shim: `gpui_launch` builds the shadow tree and
      # returns, so an application must not treat its return as a window.
      check gpui_has_window_backend() == 0'u8

suite "display_session: GPUI's Linux platform choice":

  test "WAYLAND_DISPLAY alone picks Wayland":
    check gpuiLinuxPlatform(envOf({"WAYLAND_DISPLAY": "wayland-1"})) ==
      glpWayland

  test "DISPLAY alone picks X11":
    check gpuiLinuxPlatform(envOf({"DISPLAY": ":0"})) == glpX11

  test "both set: Wayland wins, as in guess_compositor":
    check gpuiLinuxPlatform(envOf({"WAYLAND_DISPLAY": "wayland-1",
                                   "DISPLAY": ":0"})) == glpWayland

  test "neither set: headless, the platform that opens no window":
    check gpuiLinuxPlatform(envOf([])) == glpHeadless

  test "empty values count as unset":
    check gpuiLinuxPlatform(envOf({"WAYLAND_DISPLAY": "",
                                   "DISPLAY": ""})) == glpHeadless
    # An empty WAYLAND_DISPLAY does not hide a real DISPLAY.
    check gpuiLinuxPlatform(envOf({"WAYLAND_DISPLAY": "",
                                   "DISPLAY": ":1"})) == glpX11

  test "ZED_HEADLESS forces headless by presence, even when empty":
    check gpuiLinuxPlatform(envOf({"ZED_HEADLESS": "",
                                   "WAYLAND_DISPLAY": "wayland-1",
                                   "DISPLAY": ":0"})) == glpHeadless
    check gpuiLinuxPlatform(envOf({"ZED_HEADLESS": "1",
                                   "DISPLAY": ":0"})) == glpHeadless

  test "only Linux and FreeBSD choose the platform from the environment":
    check gpuiChoosesPlatformFromEnv("linux")
    check gpuiChoosesPlatformFromEnv("freebsd")
    check not gpuiChoosesPlatformFromEnv("macosx")
    check not gpuiChoosesPlatformFromEnv("windows")

  test "processEnv tells unset from empty":
    const name = "ISONIM_GPUI_TEST_WINDOW_BACKEND_PROBE"
    delEnv(name)
    check processEnv(name).isNone
    putEnv(name, "")
    check processEnv(name) == some("")
    putEnv(name, "x")
    check processEnv(name) == some("x")
    delEnv(name)
