#!/usr/bin/env bash
# Validate that all Rust extern "C" functions have corresponding Nim bindings.
#
# Usage: ./tools/check_bindings.sh
#
# Exits 0 if all exports are covered, 1 if there are mismatches.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RUST_SRC="$REPO_ROOT/rust/gpui-nim-shim/src/lib.rs"
NIM_BINDINGS="$REPO_ROOT/src/isonim_gpui/bindings.nim"

# Extract exported function names from Rust
RUST_FUNCS=$(grep -oP 'pub extern "C" fn \K\w+' "$RUST_SRC" | sort)
RUST_COUNT=$(echo "$RUST_FUNCS" | wc -l)

# Extract imported function names from Nim bindings
NIM_FUNCS=$(grep -oP '(?<=proc )\w+(?=\*)' "$NIM_BINDINGS" | sort)
NIM_COUNT=$(echo "$NIM_FUNCS" | wc -l)

echo "Rust extern \"C\" exports: $RUST_COUNT"
echo "Nim binding imports:     $NIM_COUNT"
echo ""

MISSING=$(comm -23 <(echo "$RUST_FUNCS") <(echo "$NIM_FUNCS"))
EXTRA=$(comm -13 <(echo "$RUST_FUNCS") <(echo "$NIM_FUNCS"))

STATUS=0

if [ -n "$MISSING" ]; then
    echo "MISSING in Nim bindings (present in Rust but not in Nim):"
    echo "$MISSING" | sed 's/^/  - /'
    STATUS=1
fi

if [ -n "$EXTRA" ]; then
    echo "EXTRA in Nim bindings (present in Nim but not in Rust):"
    echo "$EXTRA" | sed 's/^/  - /'
    STATUS=1
fi

if [ "$STATUS" -eq 0 ]; then
    echo "All $RUST_COUNT Rust exports have matching Nim bindings."

    # Also check the generated bindings file if it exists
    GENERATED="$REPO_ROOT/src/isonim_gpui/bindings_generated.nim"
    if [ -f "$GENERATED" ]; then
        GEN_FUNCS=$(grep -oP 'importc: "\K\w+' "$GENERATED" | sort)
        GEN_MISSING=$(comm -23 <(echo "$RUST_FUNCS") <(echo "$GEN_FUNCS"))
        if [ -n "$GEN_MISSING" ]; then
            echo ""
            echo "WARNING: bindings_generated.nim is missing:"
            echo "$GEN_MISSING" | sed 's/^/  - /'
            echo "Run 'just generate-bindings' to regenerate."
            STATUS=1
        else
            echo "Generated bindings also match ($NIM_COUNT/$RUST_COUNT)."
        fi
    fi
fi

echo ""
exit $STATUS
