#!/usr/bin/env bash
# wayland-run-test.sh - Run commands in a nested Wayland compositor
#
# Usage:
#   ./scripts/wayland-run-test.sh <command> [args...]
#   ./scripts/wayland-run-test.sh --record <command> [args...]
#   ./scripts/wayland-run-test.sh --stream <command> [args...]
#   ./scripts/wayland-run-test.sh --window <command> [args...]
#   ./scripts/wayland-run-test.sh --compositor sway <command> [args...]
#
# Options:
#   --window              Show compositor as a visible nested window
#   --record              Record the Wayland display to a video file (MP4)
#   --stream              Stream the Wayland display to a video player (mpv)
#   --compositor <name>   Force specific compositor: sway or cage
#
# ===========================================================================
# WHICH COMPOSITOR, AND WHY IT IS NO LONGER WESTON  (RS-M14b, 2026-09-17)
# ===========================================================================
#
# SWAY — the default, and the only configuration GPUI has been observed
# to render under here. `WLR_BACKENDS=headless`, verified two independent
# ways with the same byte-for-byte output: the gles2 renderer, and the
# fully software path (`WLR_RENDERER=pixman` with lavapipe via
# `VK_ICD_FILENAMES`). It advertises `wl_seat` and
# `zwlr_screencopy_manager_v1`, the latter being what lets `grim` read
# the output back for `scripts/wayland-capture-frame.sh`. Xwayland hosted
# by sway works too.
#
# WESTON — REMOVED, and it used to be the DEFAULT, which is the part that
# mattered. `weston --backend=headless-backend.so` advertises no
# `wl_seat`, and GPUI's Wayland client unwraps that `None`
# (gpui_linux/src/linux/wayland/client.rs). So every GPUI client dies at
# startup under it. This script defaulted to weston for any invocation
# that was not `--record` / `--stream` / `--window`, i.e. for exactly the
# plain `wayland-run-test.sh <cmd>` form a test lane uses — so the one
# compositor that cannot run the thing under test was the one picked by
# default. Asking for it now fails with that explanation rather than
# silently producing a broken display.
#
# XVFB — a different script (`scripts/xvfb-run-test.sh`), and THIS BLOCK
# WAS WRONG UNTIL 2026-09-22. It said: *"cannot work at all. No DRI3, so
# wgpu never gets a surface: the window reaches `IsViewable` at its
# requested size and paints nothing. That is a pass-shaped failure."*
#
# **IT PAINTS.** Re-measured from `codetracer`'s PLAT-37 lane
# (`ci/test/plat37-window-frame.sh`, `probe_configurations`), with the
# windowed shim at `gpui-pre 0.3.5`: the Xvfb framebuffer goes from 303
# non-NUL bytes of 8,297,632 with no client attached to 5,184,303 with
# `codetracer-gpui` running, and the frame read out of it is the whole
# front-end at 1440x900. `libEGL warning: DRI3 error: Could not get DRI3
# device` is still printed — that part was observed correctly — but the
# conclusion does not follow from it: wgpu falls back to a software
# Vulkan device and renders.
#
# What Xvfb genuinely cannot do is be CAPTURED the way this harness
# captures: `scripts/wayland-capture-frame.sh` runs `grim`, which speaks
# `zwlr_screencopy_manager_v1`, a Wayland protocol that does not exist on
# an X display. (`Xvfb -fbdir` reads the framebuffer directly and is what
# the PLAT-37 probe uses.) So sway remains this script's compositor
# because of what READS the screen, not because X draws nothing.
#
# THE RUNTIME DIRECTORY. In headless mode this script gives the
# compositor a PRIVATE, SHORT `XDG_RUNTIME_DIR` under /tmp, for two
# reasons, and the first one is measured.
#
# CORRECTNESS, and be precise about when the old code failed, because
# the honest statement is narrower than "it never worked". The previous
# code slept one second, diffed a glob of `$XDG_RUNTIME_DIR/wayland-*`
# against a pre-computed list, and fell back to a literal when that came
# up empty. Re-measured 2026-09-17 on this box, at load average 70-85:
#
#   * with a SHORT inherited `XDG_RUNTIME_DIR` (here `/run/user/1003`)
#     the old sway path WORKED — 8 attempts, 8 passes. The one-second
#     sleep is still an unbounded race in principle, but it did not lose
#     it once, and claiming otherwise would be the same kind of
#     unfalsifiable assertion this lane keeps finding.
#
#   * with a LONG `XDG_RUNTIME_DIR` (158 chars, i.e. any nested agent or
#     CI scratch path) it failed 100%, and the mechanism is `sun_path`:
#     sway ABORTS — "Aborted (core dumped)" — because the socket path
#     does not fit in the 108 bytes of `sockaddr_un.sun_path`. No socket
#     is created, the glob stays unexpanded, and the script reports
#     `ERROR: sway did not become ready on wayland-*`. The same
#     invocation under this script's private short directory passes.
#
# So the runtime directory is the defect, not the sleep, and a short
# private one fixes it for every caller regardless of what it inherited.
# A private directory also makes the new socket the ONLY socket, so it
# is found by looking rather than by differencing.
#
# ISOLATION, the second reason: two runs (or two agents) sharing a
# runtime directory raced over which socket was "new".
#
# `--window` mode keeps the host's runtime directory, since a nested
# compositor has to reach the host's socket to connect at all.

set -euo pipefail

RESOLUTION="${HEADLESS_GUI_RESOLUTION:-1920x1080}"
FRAMERATE="${HEADLESS_GUI_FRAMERATE:-30}"
RECORDING_DIR="${HEADLESS_GUI_RECORDING_DIR:-target/test-recordings}"
PLAYER="${HEADLESS_GUI_PLAYER:-mpv}"

RECORD="${HEADLESS_GUI_RECORD:-0}"
STREAM="${HEADLESS_GUI_STREAM:-0}"
WINDOW="${HEADLESS_GUI_WINDOW:-0}"
EXPLICIT_COMPOSITOR="${HEADLESS_GUI_COMPOSITOR:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
  --record)
    RECORD=1
    shift
    ;;
  --stream)
    STREAM=1
    shift
    ;;
  --window)
    WINDOW=1
    shift
    ;;
  --compositor)
    if [[ -z "${2:-}" ]]; then
      echo "Error: --compositor requires an argument (sway or cage)" >&2
      exit 1
    fi
    EXPLICIT_COMPOSITOR="$2"
    shift 2
    ;;
  --)
    shift
    break
    ;;
  -*)
    echo "Unknown option: $1" >&2
    echo "Usage: $0 [--window] [--record] [--stream] [--compositor <name>] [--] <command> [args...]" >&2
    exit 1
    ;;
  *)
    break
    ;;
  esac
done

if [[ $# -eq 0 ]]; then
  echo "Usage: $0 [--window] [--record] [--stream] [--compositor <name>] [--] <command> [args...]" >&2
  exit 1
fi

# Determine compositor. Sway is the default in every mode; see the
# header for what happened when weston was.
if [[ -n "$EXPLICIT_COMPOSITOR" ]]; then
  case "$EXPLICIT_COMPOSITOR" in
  sway | cage) COMPOSITOR="$EXPLICIT_COMPOSITOR" ;;
  weston)
    echo "Error: weston is not supported by this harness." >&2
    echo "" >&2
    echo "  'weston --backend=headless-backend.so' advertises no wl_seat, and" >&2
    echo "  GPUI's Wayland client unwraps that None at startup, so every GPUI" >&2
    echo "  process dies immediately under it. Measured 2026-09-17." >&2
    echo "" >&2
    echo "  Use sway (the default) or cage." >&2
    exit 1
    ;;
  *)
    echo "Error: Unknown compositor '$EXPLICIT_COMPOSITOR'. Valid: sway, cage" >&2
    exit 1
    ;;
  esac
else
  COMPOSITOR="sway"
fi

USE_HEADLESS=1
if [[ "$WINDOW" == "1" ]]; then
  USE_HEADLESS=0
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SWAY_CONFIG="${SCRIPT_DIR}/../nix/sway-headless.conf"

# Verify required tools
case "$COMPOSITOR" in
sway)
  for tool in sway wayland-info; do
    if ! command -v "$tool" &>/dev/null; then
      echo "Error: $tool not found. Ensure sway and wayland-utils are in your nix dev shell." >&2
      exit 1
    fi
  done
  if [[ ! -f "$SWAY_CONFIG" ]]; then
    echo "Error: Sway config not found at $SWAY_CONFIG" >&2
    exit 1
  fi
  ;;
cage)
  for tool in cage wayland-info; do
    if ! command -v "$tool" &>/dev/null; then
      echo "Error: $tool not found." >&2
      exit 1
    fi
  done
  ;;
esac

if [[ "$RECORD" == "1" ]] || [[ "$STREAM" == "1" ]]; then
  if ! command -v wf-recorder &>/dev/null; then
    echo "Error: wf-recorder not found. Required for video recording/streaming." >&2
    exit 1
  fi
fi

if [[ "$RECORD" == "1" ]] && [[ "$STREAM" == "1" ]]; then
  if ! command -v ffmpeg &>/dev/null; then
    echo "Error: ffmpeg not found. Required for simultaneous recording and streaming." >&2
    exit 1
  fi
fi

# --- XDG_RUNTIME_DIR --------------------------------------------------
#
# Headless: a private, SHORT directory, so the compositor's socket is the
# only one in it and can be found by looking rather than by differencing
# a glob against a pre-computed list (which is what used to fail here —
# see the header). Short because `sockaddr_un.sun_path` is 108 bytes and
# sway reports an overrun only as `Unable to open wayland socket`.
#
# Nested (`--window`): keep the host's, because sway has to reach the
# host compositor's socket to connect to it in the first place.
if [[ "$USE_HEADLESS" == "1" ]]; then
  _runtime_dir="$(mktemp -d /tmp/isonim-gpui-wl-XXXXXX)"
  export XDG_RUNTIME_DIR="$_runtime_dir"
  chmod 700 "$XDG_RUNTIME_DIR"
  CLEANUP_XDG=1
  echo "Private XDG_RUNTIME_DIR: $XDG_RUNTIME_DIR"
elif [[ -z "${XDG_RUNTIME_DIR:-}" ]] || [[ ! -d "${XDG_RUNTIME_DIR:-}" ]]; then
  _runtime_dir="$(mktemp -d /tmp/isonim-gpui-wl-XXXXXX)"
  export XDG_RUNTIME_DIR="$_runtime_dir"
  chmod 700 "$XDG_RUNTIME_DIR"
  echo "Warning: XDG_RUNTIME_DIR not set, using temp dir: $XDG_RUNTIME_DIR"
  CLEANUP_XDG=1
fi

COMPOSITOR_LOG="$XDG_RUNTIME_DIR/$COMPOSITOR.log"

# Comma-joined list of the wayland sockets already present, so
# `wait_for_socket` can tell a pre-existing one from the compositor's.
# Empty in headless mode, where the runtime dir is private and new.
existing_sockets() {
  local sock base out=""
  for sock in "$XDG_RUNTIME_DIR"/wayland-*; do
    [[ "$sock" == *.lock ]] && continue
    [[ -S "$sock" ]] || continue
    base="$(basename "$sock")"
    out="${out:+$out,}$base"
  done
  echo "$out"
}

# Wait (up to ~10 s) for a wayland socket to show up in $XDG_RUNTIME_DIR
# and echo its name. Waits for the SOCKET, not for a fixed sleep: on a
# loaded shared runner one second is not a bound on anything.
wait_for_socket() {
  local seen_before="$1" i sock base
  for ((i = 0; i < 100; i++)); do
    for sock in "$XDG_RUNTIME_DIR"/wayland-*; do
      [[ "$sock" == *.lock ]] && continue
      [[ -S "$sock" ]] || continue
      base="$(basename "$sock")"
      if [[ ",$seen_before," != *",$base,"* ]]; then
        echo "$base"
        return 0
      fi
    done
    sleep 0.1
  done
  return 1
}

# Start compositor
case "$COMPOSITOR" in
sway)
  if [[ "$USE_HEADLESS" == "1" ]]; then
    echo "Starting Sway compositor (headless)..."
    SEEN=""
    WLR_BACKENDS=headless WLR_RENDERER="${WLR_RENDERER:-pixman}" \
      sway -c "$SWAY_CONFIG" >"$COMPOSITOR_LOG" 2>&1 &
  else
    echo "Starting Sway compositor (nested window)..."
    SEEN="$(existing_sockets)"
    sway -c "$SWAY_CONFIG" >"$COMPOSITOR_LOG" 2>&1 &
  fi
  COMPOSITOR_PID=$!
  ;;

cage)
  if [[ "$USE_HEADLESS" == "1" ]]; then
    SEEN=""
    WLR_BACKENDS=headless cage -d -- sh -c "sleep 3600" >"$COMPOSITOR_LOG" 2>&1 &
  else
    SEEN="$(existing_sockets)"
    cage -d -- sh -c "sleep 3600" >"$COMPOSITOR_LOG" 2>&1 &
  fi
  COMPOSITOR_PID=$!
  ;;
esac

if ! SOCKET="$(wait_for_socket "$SEEN")"; then
  echo "ERROR: $COMPOSITOR did not create a wayland socket in $XDG_RUNTIME_DIR" >&2
  echo "--- $COMPOSITOR log ---" >&2
  cat "$COMPOSITOR_LOG" >&2 || true
  kill "$COMPOSITOR_PID" 2>/dev/null || true
  [[ "${CLEANUP_XDG:-0}" == "1" ]] && rm -rf "$XDG_RUNTIME_DIR"
  exit 1
fi
echo "$COMPOSITOR created socket: $SOCKET"

export WAYLAND_DISPLAY="$SOCKET"

cleanup() {
  local exit_code=$?

  if [[ -n "${RECORDER_PID:-}" ]]; then
    echo "Stopping video recorder..."
    kill -INT "$RECORDER_PID" 2>/dev/null || true
    for i in {1..50}; do
      if ! kill -0 "$RECORDER_PID" 2>/dev/null; then break; fi
      sleep 0.1
    done
    if kill -0 "$RECORDER_PID" 2>/dev/null; then
      kill -9 "$RECORDER_PID" 2>/dev/null || true
    fi
    wait "$RECORDER_PID" 2>/dev/null || true
  fi

  if [[ -n "${STREAM_PID:-}" ]]; then
    kill "$STREAM_PID" 2>/dev/null || true
    wait "$STREAM_PID" 2>/dev/null || true
  fi

  if [[ -n "${COMPOSITOR_PID:-}" ]]; then
    echo "Stopping $COMPOSITOR (PID $COMPOSITOR_PID)..."
    kill "$COMPOSITOR_PID" 2>/dev/null || true
    for i in {1..30}; do
      if ! kill -0 "$COMPOSITOR_PID" 2>/dev/null; then break; fi
      sleep 0.1
    done
    if kill -0 "$COMPOSITOR_PID" 2>/dev/null; then
      kill -9 "$COMPOSITOR_PID" 2>/dev/null || true
    fi
    wait "$COMPOSITOR_PID" 2>/dev/null || true
  fi

  if [[ "${CLEANUP_XDG:-0}" == "1" ]]; then
    rm -rf "$XDG_RUNTIME_DIR"
  fi

  if [[ -n "${VIDEO_FILE:-}" ]] && [[ -f "$VIDEO_FILE" ]]; then
    echo "Recording saved: $VIDEO_FILE"
    if command -v ffprobe &>/dev/null; then
      DURATION=$(ffprobe -v error -show_entries format=duration -of default=noprint_wrappers=1:nokey=1 "$VIDEO_FILE" 2>/dev/null || echo "unknown")
      SIZE=$(du -h "$VIDEO_FILE" | cut -f1)
      echo "  Duration: ${DURATION}s, Size: $SIZE"
    fi
  fi

  exit $exit_code
}

trap cleanup EXIT

# Wait for compositor to be ready
echo "Waiting for $COMPOSITOR to be ready..."
for i in {1..50}; do
  if wayland-info &>/dev/null 2>&1; then
    echo "$COMPOSITOR is ready on socket $SOCKET"
    break
  fi
  sleep 0.1
done

if ! wayland-info &>/dev/null 2>&1; then
  echo "ERROR: $COMPOSITOR did not become ready on $WAYLAND_DISPLAY" >&2
  echo "--- $COMPOSITOR log ---" >&2
  cat "$COMPOSITOR_LOG" >&2 || true
  exit 1
fi

# The two globals GPUI and the capture harness actually need, checked by
# name rather than discovered as a crash inside the client:
#
#   wl_seat                      GPUI's Wayland client unwraps this. Its
#                                absence is why weston-headless is not
#                                supported (see the header) — and the way
#                                it presented was a bare `unwrap` on None
#                                somewhere in gpui_linux, which tells the
#                                reader nothing about the compositor.
#   zwlr_screencopy_manager_v1   what `grim` uses, and therefore what
#                                `scripts/wayland-capture-frame.sh` and
#                                every pixel assertion rest on.
WL_GLOBALS="$(wayland-info 2>/dev/null || true)"
for global in wl_seat zwlr_screencopy_manager_v1; do
  if ! grep -q "'$global'" <<<"$WL_GLOBALS"; then
    echo "ERROR: $COMPOSITOR on $WAYLAND_DISPLAY advertises no $global." >&2
    if [[ "$global" == "wl_seat" ]]; then
      echo "       GPUI cannot start without it." >&2
    else
      echo "       grim cannot read the output back, so no pixel" >&2
      echo "       assertion is possible." >&2
    fi
    exit 1
  fi
done

export XDG_SESSION_TYPE=wayland
unset DISPLAY

PLAYER_ARGS="--no-terminal"
if [[ "$PLAYER" == "mpv" ]]; then
  PLAYER_ARGS="--no-config --no-terminal"
fi

# Start video capture
if [[ "$RECORD" == "1" ]] && [[ "$STREAM" == "1" ]]; then
  mkdir -p "$RECORDING_DIR"
  VIDEO_FILE="$RECORDING_DIR/wayland-test-$(date +%Y%m%d-%H%M%S).mp4"
  echo "Recording to $VIDEO_FILE and streaming to $PLAYER..."

  wf-recorder -f - -c h264 -m matroska 2>/dev/null |
    ffmpeg -f matroska -i pipe:0 \
      -c:v copy -f tee "[f=mp4]$VIDEO_FILE|[f=mpegts]pipe:1" 2>/dev/null |
    $PLAYER $PLAYER_ARGS - &
  RECORDER_PID=$!
  STREAM_PID=$RECORDER_PID

elif [[ "$RECORD" == "1" ]]; then
  mkdir -p "$RECORDING_DIR"
  VIDEO_FILE="$RECORDING_DIR/wayland-test-$(date +%Y%m%d-%H%M%S).mp4"
  echo "Recording to $VIDEO_FILE..."
  wf-recorder -f "$VIDEO_FILE" &>/dev/null &
  RECORDER_PID=$!

elif [[ "$STREAM" == "1" ]]; then
  echo "Streaming to $PLAYER..."
  wf-recorder -f - -c h264 -m matroska 2>/dev/null |
    $PLAYER $PLAYER_ARGS - &
  STREAM_PID=$!
fi

if [[ "$RECORD" == "1" ]] || [[ "$STREAM" == "1" ]]; then
  sleep 0.5
fi

echo "Running: $*"
echo "---"
"$@"
