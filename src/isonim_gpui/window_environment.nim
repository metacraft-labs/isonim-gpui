## The window environment — what the current desktop expects of an
## application's title bar (GWIN-M1).
##
## The application decides whether to draw window buttons and where; this
## module only describes the environment, so that the application can follow
## the desktop's preference without knowing each desktop itself:
##
##   let env = windowEnvironment(launchWindow())
##   if env.clientDecorated:
##     for b in env.preferredLeft: drawButtonOnTheLeft(b)
##     for b in env.preferredRight: drawButtonOnTheRight(b)
##
## Read it again from `onWindowState`: the decoration mode, the supported
## controls and the configured layout can all change while the window is
## open.
##
## `resolveWindowEnvironment` is the whole policy, as a pure function of its
## inputs, so it is tested without a window or a compositor; `windowEnvironment`
## only gathers those inputs from the live window and the process
## environment.

import std/[os, strutils]
import isonim_gpui/[display_session, window]

type
  WindowButton* = enum
    ## A title-bar button. The names match GPUI's `WindowButton::id` and
    ## GNOME's `button-layout` vocabulary.
    wbMinimize = "minimize"
    wbMaximize = "maximize"
    wbClose = "close"

  PlacementSource* = enum
    psPortal = "portal"
      ## The desktop's configured layout, read by GPUI through the settings
      ## portal (`gtk-decoration-layout` / GNOME `button-layout`).
    psPlatformDefault = "platform-default"
      ## No layout is known to come from the desktop (none was reported, or
      ## the portal has not answered yet); this is the platform's convention.

  WindowEnvironment* = object
    platform*: string
      ## "linux-wayland", "linux-x11", "macos", "windows" or the OS name.
    desktop*: string
      ## The first entry of `XDG_CURRENT_DESKTOP` ("GNOME", "KDE", "sway"),
      ## "" when unknown or not on a freedesktop platform.
    clientDecorated*: bool
      ## The application is expected to draw its own title bar and buttons.
    supported*: set[WindowButton]
      ## Buttons whose action the platform supports. `wbClose` always is.
    preferredLeft*, preferredRight*: seq[WindowButton]
      ## The preferred placement, filtered by `supported`, in order.
    placementSource*: PlacementSource
      ## Whether the placement was read or is the platform default, so an
      ## application or a test can tell a configured preference from a guess.
    titleBarHeight*: float
      ## The platform's conventional title-bar height in logical pixels. A
      ## hint; an application with its own toolbar uses its own height.

proc currentPlatform*(): string =
  ## The platform GPUI runs on, as the strings `WindowEnvironment.platform`
  ## uses. On Linux GPUI picks Wayland when `WAYLAND_DISPLAY` is set and X11
  ## otherwise, and so does this — through `display_session`, the one place
  ## that rule is written down. (Its headless outcome reads as "linux-x11"
  ## here, as it always has: there is no window to describe in that case.)
  when defined(macosx): "macos"
  elif defined(windows): "windows"
  elif defined(linux) or defined(freebsd):
    if gpuiLinuxPlatform(processEnv) == glpWayland: "linux-wayland"
    else: "linux-x11"
  else: hostOS

proc currentDesktop*(): string =
  ## The first entry of `XDG_CURRENT_DESKTOP` (a colon-separated list, e.g.
  ## "ubuntu:GNOME"; see the Desktop Entry Specification,
  ## https://specifications.freedesktop.org/desktop-entry-spec/latest/recognized-keys.html).
  getEnv("XDG_CURRENT_DESKTOP").split(':')[0].strip()

proc parseButtonLayout*(layout: string): tuple[ok: bool; left, right: seq[WindowButton]] =
  ## Parse GNOME's `button-layout` syntax: `left:right`, each side a
  ## comma-separated list. Unknown names (`appmenu`, `icon`, `spacer`) are
  ## skipped, as GTK does; repeated buttons keep their first position. A
  ## layout without a colon is not one, and `ok` is false.
  let colon = layout.find(':')
  if colon < 0: return (false, @[], @[])
  var seen: set[WindowButton]
  proc side(text: string): seq[WindowButton] =
    for name in text.split(','):
      for b in WindowButton:
        if name.strip() == $b and b notin seen:
          seen.incl b
          result.add b
  let left = side(layout[0 ..< colon])
  let right = side(layout[colon + 1 .. ^1])
  (true, left, right)

proc platformDefaultLayout*(platform: string): tuple[left, right: seq[WindowButton]] =
  ## The convention where no layout is reported: on macOS close, minimize,
  ## maximize on the left; elsewhere minimize, maximize, close on the right
  ## (GPUI's `WindowButtonLayout::linux_default`, and Windows'). GNOME's own
  ## default, `appmenu:close`, is a configured layout and arrives through the
  ## portal like any other.
  if platform == "macos": (@[wbClose, wbMinimize, wbMaximize], @[])
  else: (@[], @[wbMinimize, wbMaximize, wbClose])

proc conventionalTitleBarHeight*(platform: string): float =
  case platform
  of "macos": 28.0
  else: 32.0

proc resolveWindowEnvironment*(platform, desktop: string; clientDecorated: bool;
                               controls: set[WindowControl];
                               buttonLayout: string): WindowEnvironment =
  ## The policy: which buttons the platform supports, and where the desktop
  ## wants them. Pure; `windowEnvironment` feeds it the live values.
  result = WindowEnvironment(platform: platform, desktop: desktop,
                             clientDecorated: clientDecorated,
                             titleBarHeight: conventionalTitleBarHeight(platform))
  result.supported = {wbClose}
  if wcMinimize in controls: result.supported.incl wbMinimize
  if wcMaximize in controls: result.supported.incl wbMaximize
  let parsed = parseButtonLayout(buttonLayout)
  var left, right: seq[WindowButton]
  if parsed.ok:
    (left, right) = (parsed.left, parsed.right)
    result.placementSource = psPortal
  else:
    (left, right) = platformDefaultLayout(platform)
    result.placementSource = psPlatformDefault
  for b in left:
    if b in result.supported: result.preferredLeft.add b
  for b in right:
    if b in result.supported: result.preferredRight.add b

proc windowEnvironment*(win: GpuiWindow): WindowEnvironment =
  ## The environment of a live window (usually `launchWindow()`).
  resolveWindowEnvironment(currentPlatform(), currentDesktop(),
                           win.clientDecorated, win.supportedControls,
                           win.buttonLayout)
