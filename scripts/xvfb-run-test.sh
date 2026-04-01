#!/usr/bin/env bash
# xvfb-run-test.sh - Run commands in a virtual X11 display (Xvfb)
#
# Usage:
#   ./scripts/xvfb-run-test.sh <command> [args...]
#   ./scripts/xvfb-run-test.sh --record <command> [args...]
#   ./scripts/xvfb-run-test.sh --stream <command> [args...]
#
# Options:
#   --record    Record the X11 display to a video file (MP4)
#   --stream    Stream the X11 display to a video player (mpv)
#
# Environment variables:
#   HEADLESS_GUI_RECORD=1          Enable video recording (same as --record)
#   HEADLESS_GUI_STREAM=1          Enable video streaming (same as --stream)
#   HEADLESS_GUI_RECORDING_DIR     Directory for recordings (default: target/test-recordings)
#   HEADLESS_GUI_PLAYER            Video player for streaming (default: mpv)
#   HEADLESS_GUI_RESOLUTION        Display resolution (default: 1920x1080)
#   HEADLESS_GUI_DEPTH             Color depth (default: 24)
#   HEADLESS_GUI_FRAMERATE         Video framerate (default: 30)

set -euo pipefail

# Configuration with defaults
RESOLUTION="${HEADLESS_GUI_RESOLUTION:-1920x1080}"
DEPTH="${HEADLESS_GUI_DEPTH:-24}"
FRAMERATE="${HEADLESS_GUI_FRAMERATE:-30}"
RECORDING_DIR="${HEADLESS_GUI_RECORDING_DIR:-target/test-recordings}"
PLAYER="${HEADLESS_GUI_PLAYER:-mpv}"

# Parse options
RECORD="${HEADLESS_GUI_RECORD:-0}"
STREAM="${HEADLESS_GUI_STREAM:-0}"

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
  --)
    shift
    break
    ;;
  -*)
    echo "Unknown option: $1" >&2
    echo "Usage: $0 [--record] [--stream] [--] <command> [args...]" >&2
    exit 1
    ;;
  *)
    break
    ;;
  esac
done

if [[ $# -eq 0 ]]; then
  echo "Usage: $0 [--record] [--stream] [--] <command> [args...]" >&2
  exit 1
fi

# Check if we're on Linux (Xvfb is Linux-only)
if [[ "$(uname -s)" != "Linux" ]]; then
  echo "Warning: Xvfb is only available on Linux. Running command directly." >&2
  exec "$@"
fi

# Check if Xvfb binary is available
if ! command -v Xvfb &>/dev/null; then
  echo "Warning: Xvfb not found in PATH. Running command without X11 display." >&2
  echo "   GUI tests will be skipped. Ensure xorg.xorgserver is in your nix dev shell." >&2
  exec "$@"
fi

# Quick smoke test: verify Xvfb can actually start
_smoke_test_xvfb() {
  local test_display=":$((200 + RANDOM % 50))"
  local xvfb_pid

  if Xvfb "$test_display" -screen 0 640x480x24 -nolisten tcp &>/dev/null & then
    xvfb_pid=$!
    sleep 0.3

    if kill -0 "$xvfb_pid" 2>/dev/null; then
      kill "$xvfb_pid" 2>/dev/null || true
      wait "$xvfb_pid" 2>/dev/null || true
      return 0
    fi
  fi
  return 1
}

if ! _smoke_test_xvfb; then
  echo "Warning: Xvfb binary found but failed to start. Running command without X11 display." >&2
  exec "$@"
fi

# Verify recording/streaming tools if needed
if [[ "$RECORD" == "1" ]] || [[ "$STREAM" == "1" ]]; then
  if ! command -v ffmpeg &>/dev/null; then
    echo "Error: ffmpeg not found. Required for video recording/streaming." >&2
    exit 1
  fi
fi

if [[ "$STREAM" == "1" ]]; then
  if ! command -v "$PLAYER" &>/dev/null; then
    echo "Warning: $PLAYER not found. Streaming may not work." >&2
  fi
fi

# GL / display environment
export XDG_SESSION_TYPE=x11
export GDK_BACKEND=x11
unset WAYLAND_DISPLAY

# Enable software OpenGL rendering for Xvfb (no hardware GPU)
export LIBGL_ALWAYS_SOFTWARE=1

# Ensure mesa driver paths are set for software GL rendering
if [[ -z "${LIBGL_DRIVERS_PATH:-}" ]]; then
  _mesa_dri=$(ls -d /nix/store/*-mesa-*/lib/dri 2>/dev/null | head -1)
  if [[ -n "$_mesa_dri" ]]; then
    export LIBGL_DRIVERS_PATH="$_mesa_dri"
    _mesa_lib=$(dirname "$_mesa_dri")
    _libglvnd_lib=$(dirname "$(ls /nix/store/*-libglvnd-*/lib/libGL.so.1 2>/dev/null | head -1)")
    export LD_LIBRARY_PATH="${_mesa_lib}:${_libglvnd_lib:+$_libglvnd_lib:}${LD_LIBRARY_PATH:-}"
    echo "Resolved mesa GL paths: LIBGL_DRIVERS_PATH=$LIBGL_DRIVERS_PATH"
  fi
fi

# Find a free display number
DISPLAY_START=$((99 + RANDOM % 52))
DISPLAY_NUM=$DISPLAY_START
while [ -e "/tmp/.X${DISPLAY_NUM}-lock" ] || [ -e "/tmp/.X11-unix/X${DISPLAY_NUM}" ]; do
  DISPLAY_NUM=$((DISPLAY_NUM + 1))
  if [[ $DISPLAY_NUM -gt 200 ]]; then
    DISPLAY_NUM=99
  fi
  if [[ $DISPLAY_NUM -eq $DISPLAY_START ]]; then
    echo "Error: Could not find a free display number (tried :99-:200)" >&2
    exit 1
  fi
done

echo "Starting Xvfb on display :$DISPLAY_NUM with resolution ${RESOLUTION}x${DEPTH}..."

Xvfb ":$DISPLAY_NUM" -screen 0 "${RESOLUTION}x${DEPTH}" -nolisten tcp &
XVFB_PID=$!

export DISPLAY=":$DISPLAY_NUM"

cleanup() {
  local exit_code=$?

  if [[ -n "${FFMPEG_PID:-}" ]]; then
    echo "Stopping video capture..."
    kill -INT "$FFMPEG_PID" 2>/dev/null || true
    wait "$FFMPEG_PID" 2>/dev/null || true
  fi

  if [[ -n "${STREAM_PID:-}" ]]; then
    kill "$STREAM_PID" 2>/dev/null || true
    wait "$STREAM_PID" 2>/dev/null || true
  fi

  if [[ -n "${XVFB_PID:-}" ]]; then
    echo "Stopping Xvfb (PID $XVFB_PID)..."
    kill "$XVFB_PID" 2>/dev/null || true
    wait "$XVFB_PID" 2>/dev/null || true
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

# Wait for Xvfb to be ready (up to 5 seconds)
echo "Waiting for Xvfb to be ready..."
for i in {1..50}; do
  if xdpyinfo &>/dev/null 2>&1; then
    echo "Xvfb is ready on display :$DISPLAY_NUM"
    break
  fi
  sleep 0.1
done

if ! xdpyinfo &>/dev/null 2>&1; then
  echo "ERROR: Xvfb did not become ready on $DISPLAY" >&2
  exit 1
fi

# Build player command line arguments
PLAYER_ARGS="--no-terminal"
if [[ "$PLAYER" == "mpv" ]]; then
  PLAYER_ARGS="--no-config --no-terminal"
fi

# Start video capture if requested
if [[ "$RECORD" == "1" ]] && [[ "$STREAM" == "1" ]]; then
  mkdir -p "$RECORDING_DIR"
  VIDEO_FILE="$RECORDING_DIR/test-$(date +%Y%m%d-%H%M%S).mp4"
  echo "Recording to $VIDEO_FILE and streaming to $PLAYER..."

  ffmpeg -f x11grab -video_size "$RESOLUTION" -framerate "$FRAMERATE" -i "$DISPLAY" \
    -c:v libx264 -preset ultrafast -tune zerolatency \
    -f tee "[f=mp4]$VIDEO_FILE|[f=mpegts]pipe:1" 2>/dev/null |
    $PLAYER $PLAYER_ARGS - &
  FFMPEG_PID=$!
  STREAM_PID=$FFMPEG_PID

elif [[ "$RECORD" == "1" ]]; then
  mkdir -p "$RECORDING_DIR"
  VIDEO_FILE="$RECORDING_DIR/test-$(date +%Y%m%d-%H%M%S).mp4"
  FFMPEG_LOG="$RECORDING_DIR/ffmpeg-$(date +%Y%m%d-%H%M%S).log"
  echo "Recording to $VIDEO_FILE..."

  ffmpeg -f x11grab -video_size "$RESOLUTION" -framerate "$FRAMERATE" -i "$DISPLAY" \
    -c:v libx264 -preset ultrafast -pix_fmt yuv420p \
    "$VIDEO_FILE" 2>"$FFMPEG_LOG" &
  FFMPEG_PID=$!

elif [[ "$STREAM" == "1" ]]; then
  echo "Streaming to $PLAYER..."

  ffmpeg -f x11grab -video_size "$RESOLUTION" -framerate "$FRAMERATE" -i "$DISPLAY" \
    -c:v libx264 -preset ultrafast -tune zerolatency \
    -f mpegts pipe:1 2>/dev/null |
    $PLAYER $PLAYER_ARGS - &
  STREAM_PID=$!
fi

if [[ "$RECORD" == "1" ]] || [[ "$STREAM" == "1" ]]; then
  sleep 0.5
  if [[ -n "${FFMPEG_PID:-}" ]] && ! kill -0 "$FFMPEG_PID" 2>/dev/null; then
    echo "ERROR: ffmpeg failed to start. Check log: ${FFMPEG_LOG:-'(no log)'}" >&2
    exit 1
  fi
fi

echo "Running: $*"
echo "---"
"$@"
