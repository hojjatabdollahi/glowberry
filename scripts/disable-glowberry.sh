#!/usr/bin/env bash
# Disable GlowBerry and restore original cosmic-bg

set -e

SYMLINK_PATH="/usr/local/bin/cosmic-bg"

echo "Disabling GlowBerry override..."

# Remove the symlink if it exists
if [ -L "$SYMLINK_PATH" ]; then
    sudo rm "$SYMLINK_PATH"
    echo "Removed $SYMLINK_PATH symlink"
else
    echo "No symlink found at $SYMLINK_PATH"
fi

# Kill any running cosmic-bg/glowberry processes
echo ""
echo "Killing running background processes..."
pkill -x cosmic-bg 2>/dev/null || true
pkill -x glowberry 2>/dev/null || true

# Verify
echo ""
echo "Verification:"
echo "  which cosmic-bg: $(which cosmic-bg 2>/dev/null || echo 'not found')"
if [ -f /usr/bin/cosmic-bg ]; then
    echo "  /usr/bin/cosmic-bg exists"
fi

echo ""
echo "Original cosmic-bg at /usr/bin/cosmic-bg is now active!"
echo "Log out and back in, or the background service will restart automatically."
