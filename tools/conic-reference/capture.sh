#!/usr/bin/env bash
# capture.sh — regenerate tests/fixtures/conic/browser-strip.ppm, the browser
# reference `tests/test_gui_conic.nim` compares the shim's conic gradient
# with. Needs Node and a Playwright install with its Chromium: run it inside a
# shell that sets PLAYWRIGHT_BROWSERS_PATH (CodeTracer's dev shell does) and
# pass the `playwright` module directory, e.g.
#
#   tools/conic-reference/capture.sh ../codetracer/node_modules/playwright
#
# The fixture committed was produced by Chromium 141.0.7390.37 (Playwright's
# chromium-1194 build, headless), device scale 1.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
pw="$(cd "${1:?usage: capture.sh <playwright module dir>}" && pwd)"
node "${here}/capture.mjs" "${pw}" "${here}/../../tests/fixtures/conic/browser-strip.ppm"
