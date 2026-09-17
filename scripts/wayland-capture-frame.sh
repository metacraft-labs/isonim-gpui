#!/usr/bin/env bash
# wayland-capture-frame.sh — capture one settled frame of the Wayland
# output into a binary PPM, then signal that the capture is over.
#
# Usage: wayland-capture-frame.sh <out.ppm> [timeout-seconds]
#
# Written for the windowed pixel case in `tests/test_gui.nim`, which
# cannot do this itself: it is blocked inside `gpui_launch`, i.e. inside
# GPUI's platform event loop, for as long as the window it wants a
# picture of exists. So the picture has to be taken from outside the
# process, and the process has to be told when it may stop.
#
# The protocol is two files:
#
#   <out.ppm>        written ONLY if a frame was actually captured
#   <out.ppm>.done   written ALWAYS, exactly once, as the last act
#
# `.done` is unconditional on purpose. The test's watcher thread waits
# for it and then calls `gpui_quit()`; if this script could exit without
# writing it, a capture failure would present as a hung test rather than
# as a failed one. The absence of <out.ppm> next to a present `.done` is
# what the test reads as "nothing was captured", and it fails on it.
#
# WHEN IS A FRAME READY?
#
# In two phases, because the caller's test binary may have opened and
# closed other windows just before this one. Measured 2026-09-17: a
# single-phase "first non-blank frame" gate captured the TAIL of the
# previous case's window teardown 846 ms in — a frame of 2,073,600 black
# pixels — and every colour assertion in the test went red on it. Red
# rather than green, which is the harness working; but the gate was
# picking the wrong moment, so it was fixed.
#
#   PHASE 1 — WAIT FOR THE OUTPUT TO GO BLANK. A headless sway output's
#   background is solid #000000, so once whatever was there has gone,
#   the payload is very nearly all NUL. Entering phase 2 only from a
#   blank screen means the frame phase 2 sees can only have been drawn
#   by the window the test is about. Bounded: if the screen never goes
#   blank we say so in this log and go on anyway, because the test's own
#   "no colour other than the scene's" assertion will fail on a dirty
#   screen, and a failed assertion beats a harness that gives up.
#
#   PHASE 2 — WAIT FOR A PAINTED, SETTLED FRAME. Painted: at least
#   PAINTED_PERCENT of the payload bytes are non-NUL. Settled: two
#   consecutive captures byte-identical, so a half-mapped surface is not
#   mistaken for a finished composition.
#
# THE PAINTED FLOOR IS A SYNCHRONISATION CONDITION, NOT THE ASSERTION,
# and the distinction is what keeps this from being circular. It says
# only "a lot of the screen is no longer black". The test then checks
# exact pixel counts and exact bounding boxes for each colour — so a
# frame that clears this gate while showing the wrong thing goes red,
# which is the whole point of the exercise.
#
# It does impose one requirement on the caller: THE SCENE MUST BE
# PREDOMINANTLY BRIGHT. The scene in `tests/test_gui.nim` fills the
# output with #ff00ff, which is two non-NUL bytes in every three, i.e.
# ~66%. Measured on the same box: blank frame 17 bytes (the ASCII
# `P6 1920 1080 255` header), a torn-down window's transient 7,740
# bytes (0.1%), the scene 4,147,217 bytes (66%). 25% sits in the middle
# of a three-decade gap. A predominantly dark scene would need a
# different gate — say, waiting on `swaymsg -t get_tree` for the view.

set -uo pipefail

OUT="${1:?usage: wayland-capture-frame.sh <out.ppm> [timeout-seconds]}"
TIMEOUT_S="${2:-30}"

# Everything this script says goes to a log beside the capture, so the
# caller can start it with inherited streams without interleaving into
# the test report, and can print the log when the case fails.
exec >>"${OUT}.log" 2>&1

# shellcheck disable=SC2329  # invoked via the EXIT trap below
finish() {
  : >"${OUT}.done"
}
trap finish EXIT

echo "wayland-capture-frame: out=$OUT timeout=${TIMEOUT_S}s WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-<unset>}"

if ! command -v grim &>/dev/null; then
  echo "grim not found in PATH — cannot capture."
  exit 1
fi
if [[ -z "${WAYLAND_DISPLAY:-}" ]]; then
  echo "WAYLAND_DISPLAY is unset — no compositor to capture from."
  exit 1
fi

# A blank 1920x1080 frame measures 17 non-NUL bytes (the ASCII header);
# 0.5% of the payload is three orders of magnitude above that and three
# orders below a painted frame.
BLANK_PERCENT=0.5
PAINTED_PERCENT=25

# 0.25s per tick.
TICK_S=0.25
BLANK_TICKS=20                      # 5 s to observe a blank screen
PAINT_TICKS=$((TIMEOUT_S * 4))

PREV="${OUT}.prev"
CUR="${OUT}.cur"
rm -f "$OUT" "$PREV" "$CUR"

# Echoes "<non-NUL bytes> <payload bytes>" for the frame in $CUR, or
# nothing at all if grim failed.
measure() {
  grim -t ppm "$CUR" 2>/dev/null || return 1
  local total nonzero
  total=$(wc -c <"$CUR")
  nonzero=$(LC_ALL=C tr -d '\0' <"$CUR" | wc -c)
  echo "$nonzero $total"
}

# percent_at_least <nonzero> <total> <percent>  — integer arithmetic on
# tenths so BLANK_PERCENT can be fractional.
percent_at_least() {
  local nonzero=$1 total=$2 percent=$3
  local tenths=${percent/./}
  [[ "$percent" == *.* ]] || tenths=$((percent * 10))
  [[ $((nonzero * 1000)) -ge $((total * tenths)) ]]
}

# --- Phase 1: wait for the output to be blank ------------------------
blanked=0
for ((i = 1; i <= BLANK_TICKS; i++)); do
  if m=$(measure); then
    read -r nonzero total <<<"$m"
    if ! percent_at_least "$nonzero" "$total" "$BLANK_PERCENT"; then
      echo "phase1 tick $i: output is blank ($nonzero/$total non-NUL) — watching for the window"
      blanked=1
      break
    fi
    echo "phase1 tick $i: output still has content ($nonzero/$total non-NUL)"
  else
    echo "phase1 tick $i: grim failed"
  fi
  sleep "$TICK_S"
done
if [[ "$blanked" -eq 0 ]]; then
  echo "phase1: output never went blank in $((BLANK_TICKS / 4))s; continuing anyway."
  echo "        the test asserts that nothing but its own scene is on screen,"
  echo "        so a dirty screen surfaces there rather than being hidden here."
fi

# --- Phase 2: wait for a painted, settled frame ----------------------
for ((i = 1; i <= PAINT_TICKS; i++)); do
  if m=$(measure); then
    read -r nonzero total <<<"$m"
    if percent_at_least "$nonzero" "$total" "$PAINTED_PERCENT"; then
      if [[ -f "$PREV" ]] && cmp -s "$PREV" "$CUR"; then
        echo "phase2 tick $i: painted and settled ($nonzero/$total non-NUL) — capturing"
        mv "$CUR" "$OUT"
        rm -f "$PREV"
        exit 0
      fi
      echo "phase2 tick $i: painted ($nonzero/$total non-NUL), waiting for it to settle"
      mv "$CUR" "$PREV"
    else
      echo "phase2 tick $i: not painted yet ($nonzero/$total non-NUL)"
      rm -f "$PREV"
    fi
  else
    echo "phase2 tick $i: grim failed"
  fi
  sleep "$TICK_S"
done

echo "timed out after ${TIMEOUT_S}s without a painted, settled frame"
# Keep the last frame we saw, under a DIFFERENT name. The caller reads
# the absence of $OUT as "nothing was captured" and fails on it; this is
# the evidence for why, and putting it at $OUT would turn a timeout into
# a picture the test would then happily analyse.
if [[ -f "$CUR" ]]; then
  mv "$CUR" "${OUT}.timeout.ppm"
  echo "last frame kept at ${OUT}.timeout.ppm"
fi
rm -f "$PREV" "$CUR"
exit 1
