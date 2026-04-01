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
#   --compositor <name>   Force specific compositor: weston, sway, or cage

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
      echo "Error: --compositor requires an argument (weston, sway, or cage)" >&2
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

# Determine compositor
if [[ -n "$EXPLICIT_COMPOSITOR" ]]; then
  case "$EXPLICIT_COMPOSITOR" in
  weston | sway | cage) COMPOSITOR="$EXPLICIT_COMPOSITOR" ;;
  *)
    echo "Error: Unknown compositor '$EXPLICIT_COMPOSITOR'. Valid: weston, sway, cage" >&2
    exit 1
    ;;
  esac
elif [[ "$RECORD" == "1" ]] || [[ "$STREAM" == "1" ]] || [[ "$WINDOW" == "1" ]]; then
  COMPOSITOR="sway"
else
  COMPOSITOR="weston"
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
weston)
  for tool in weston wayland-info; do
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

# XDG_RUNTIME_DIR
if [[ -z "${XDG_RUNTIME_DIR:-}" ]] || [[ ! -d "${XDG_RUNTIME_DIR:-}" ]]; then
  export XDG_RUNTIME_DIR="$(mktemp -d)"
  echo "Warning: XDG_RUNTIME_DIR not set, using temp dir: $XDG_RUNTIME_DIR"
  CLEANUP_XDG=1
fi

# Start compositor
case "$COMPOSITOR" in
sway)
  if [[ "$USE_HEADLESS" == "1" ]]; then
    echo "Starting Sway compositor (headless)..."
  else
    echo "Starting Sway compositor (nested window)..."
  fi

  EXISTING_SOCKETS=$(ls "$XDG_RUNTIME_DIR"/wayland-* 2>/dev/null | grep -v '\.lock$' || true)

  if [[ "$USE_HEADLESS" == "1" ]]; then
    WLR_BACKENDS=headless WLR_RENDERER=pixman sway -c "$SWAY_CONFIG" &>/dev/null &
  else
    sway -c "$SWAY_CONFIG" &
  fi
  COMPOSITOR_PID=$!
  sleep 1

  NEW_SOCKET=""
  for sock in "$XDG_RUNTIME_DIR"/wayland-*; do
    [[ "$sock" == *.lock ]] && continue
    if ! echo "$EXISTING_SOCKETS" | grep -q "^${sock}$"; then
      NEW_SOCKET=$(basename "$sock")
      break
    fi
  done
  SOCKET="${NEW_SOCKET:-wayland-0}"
  echo "Sway created socket: $SOCKET"
  ;;

cage)
  EXISTING_SOCKETS=$(ls "$XDG_RUNTIME_DIR"/wayland-* 2>/dev/null | grep -v '\.lock$' || true)
  if [[ "$USE_HEADLESS" == "1" ]]; then
    WLR_BACKENDS=headless cage -d -- sh -c "sleep 3600" &>/dev/null &
  else
    cage -d -- sh -c "sleep 3600" &
  fi
  COMPOSITOR_PID=$!
  sleep 1

  NEW_SOCKET=""
  for sock in "$XDG_RUNTIME_DIR"/wayland-*; do
    [[ "$sock" == *.lock ]] && continue
    if ! echo "$EXISTING_SOCKETS" | grep -q "^${sock}$"; then
      NEW_SOCKET=$(basename "$sock")
      break
    fi
  done
  SOCKET="${NEW_SOCKET:-wayland-0}"
  ;;

weston)
  SOCKET="wayland-test-$$"
  if [[ "$USE_HEADLESS" == "1" ]]; then
    echo "Starting Weston compositor (headless) on socket $SOCKET..."
    weston --backend=headless-backend.so --socket="$SOCKET" &>/dev/null &
  else
    echo "Starting Weston compositor (nested window) on socket $SOCKET..."
    weston --backend=wayland-backend.so --socket="$SOCKET" &
  fi
  COMPOSITOR_PID=$!
  ;;
esac

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
  exit 1
fi

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
