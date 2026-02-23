#!/usr/bin/env bash
# Generate cargo-sources.json for offline Flatpak builds.
#
# This script reads the root Cargo.lock and produces a JSON file that
# flatpak-builder uses to download all crate and git dependencies.
#
# Prerequisites:
#   pip install flatpak-cargo-generator
#
# Usage:
#   ./flatpak/generate-sources.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
OUTPUT="$SCRIPT_DIR/cargo-sources.json"

# Find the generator
GENERATOR=""
for candidate in \
    "$(command -v flatpak-cargo-generator 2>/dev/null || true)" \
    "$HOME/.local/bin/flatpak-cargo-generator" \
    ; do
    if [ -n "$candidate" ] && [ -x "$candidate" ]; then
        GENERATOR="$candidate"
        break
    fi
done

# Try pip-installed locations
if [ -z "$GENERATOR" ]; then
    GENERATOR="$(python3 -c 'import shutil; print(shutil.which("flatpak-cargo-generator") or "")' 2>/dev/null || true)"
fi

if [ -z "$GENERATOR" ]; then
    echo "Error: flatpak-cargo-generator not found."
    echo "Install it with: pip install flatpak-cargo-generator"
    exit 1
fi

echo "Using generator: $GENERATOR"
echo "Reading: $PROJECT_ROOT/Cargo.lock"
echo "Writing: $OUTPUT"

"$GENERATOR" "$PROJECT_ROOT/Cargo.lock" -o "$OUTPUT"

echo ""
echo "Generated $(wc -l < "$OUTPUT") lines in cargo-sources.json"
echo "Done."
