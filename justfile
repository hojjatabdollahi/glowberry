name := 'glowberry'
settings-name := 'glowberry-settings'
export APPID := 'io.github.hojjatabdollahi.glowberry'
settings-appid := 'io.github.hojjatabdollahi.glowberry-settings'

# Use mold linker if clang and mold exists.
clang-path := `which clang || true`
mold-path := `which mold || true`

linker-arg := if clang-path != '' {
    if mold-path != '' {
        '-C linker=' + clang-path + ' -C link-arg=--ld-path=' + mold-path + ' '
    } else {
        ''
    }
} else {
    ''
}

export RUSTFLAGS := linker-arg + env_var_or_default('RUSTFLAGS', '')

rootdir := ''
prefix := env_var('HOME') / '.local'


base-dir := absolute_path(clean(rootdir / prefix))

export INSTALL_DIR := base-dir / 'share'

cargo-target-dir := env('CARGO_TARGET_DIR', 'target')
bin-src := cargo-target-dir / 'release' / name
bin-dst := base-dir / 'bin' / name
settings-bin-src := cargo-target-dir / 'release' / settings-name
settings-bin-dst := base-dir / 'bin' / settings-name
shaders-dir := base-dir / 'share' / 'glowberry' / 'shaders'

# Helper script install location
switch-script-dst := base-dir / 'bin' / 'glowberry-switch'

# cosmic-bg symlink location
cosmic-bg-link := base-dir / 'bin' / 'cosmic-bg'

# Settings app data locations
settings-desktop-src := 'apps' / settings-name / 'data' / settings-appid + '.desktop'
settings-desktop-dst := base-dir / 'share' / 'applications' / settings-appid + '.desktop'
settings-icon-src := 'apps' / settings-name / 'data' / 'icons' / settings-appid + '.svg'
settings-icon-dst := base-dir / 'share' / 'icons' / 'hicolor' / 'scalable' / 'apps' / settings-appid + '.svg'
settings-symbolic-src := 'apps' / settings-name / 'data' / 'icons' / settings-appid + '-symbolic.svg'
settings-symbolic-dst := base-dir / 'share' / 'icons' / 'hicolor' / 'symbolic' / 'apps' / settings-appid + '-symbolic.svg'

# Default recipe which runs `just build-release`
default: build-release

# Runs `cargo clean`
clean:
    cargo clean

# `cargo clean` and removes vendored dependencies
clean-dist: clean
    rm -rf .cargo vendor vendor.tar

# Compiles with debug profile
build-debug *args:
    cargo build --workspace {{args}}

# Compiles with release profile
build-release *args: (build-debug '--release' args)

# Compiles release profile with vendored dependencies
build-vendored *args: vendor-extract (build-release '--frozen --offline' args)

# Runs a clippy check
check *args:
    cargo clippy --all-features {{args}} -- -W clippy::pedantic

# Runs a clippy check with JSON message format
check-json: (check '--message-format=json')

# Run with debug logs
run *args:
    env RUST_LOG=debug RUST_BACKTRACE=1 cargo run --release {{args}}

# Run settings app with debug logs
run-settings *args:
    env RUST_LOG=debug RUST_BACKTRACE=1 cargo run --release -p glowberry-settings {{args}}

# Installs all files (daemon + settings app)
install: _check-sudo install-daemon install-settings
    @echo ""
    @echo "=========================================="
    @echo "  GlowBerry installed successfully!"
    @echo "=========================================="
    @echo ""
    @echo "GlowBerry has been installed to ~/.local/bin/glowberry"
    @echo "A symlink has been created: ~/.local/bin/cosmic-bg -> glowberry"
    @echo ""
    @echo "Make sure ~/.local/bin is in your PATH (before /usr/bin)."
    @echo "You can also enable/disable from glowberry-settings."
    @echo ""

# Installs only the daemon
install-daemon:
    install -Dm0755 {{bin-src}} {{bin-dst}}
    ln -sf {{bin-dst}} {{cosmic-bg-link}}
    @just data/install
    @just data/icons/install
    # Install bundled shaders for live wallpapers
    install -d {{shaders-dir}}
    install -Dm0644 examples/*.wgsl {{shaders-dir}}/
    # Install the switch helper script
    install -Dm0755 scripts/glowberry-switch {{switch-script-dst}}

# Installs only the settings app
install-settings:
    install -Dm0755 {{settings-bin-src}} {{settings-bin-dst}}
    install -Dm0644 {{settings-desktop-src}} {{settings-desktop-dst}}
    install -Dm0644 {{settings-icon-src}} {{settings-icon-dst}}
    install -Dm0644 {{settings-symbolic-src}} {{settings-symbolic-dst}}

# Uninstalls all installed files
uninstall: _check-glowberry-disabled uninstall-daemon uninstall-settings

# Warn if running with sudo since local install works without it
_check-sudo:
    #!/usr/bin/env bash
    if [ "$(id -u)" -eq 0 ]; then
        echo "WARNING: You are running 'just install' as root (sudo)."
        echo "GlowBerry installs to ~/.local by default and does not need sudo."
        echo ""
        read -p "Are you sure you want to continue as root? [y/N] " answer
        if [ "$answer" != "y" ] && [ "$answer" != "Y" ]; then
            echo "Aborted. Run without sudo: just install"
            exit 1
        fi
    fi

# Check if GlowBerry override is disabled before uninstalling
_check-glowberry-disabled:
    #!/usr/bin/env bash
    if [ -L "{{cosmic-bg-link}}" ]; then
        TARGET=$(readlink "{{cosmic-bg-link}}")
        if echo "$TARGET" | grep -q glowberry; then
            echo "Removing cosmic-bg symlink at {{cosmic-bg-link}}"
            rm -f "{{cosmic-bg-link}}"
        fi
    fi

# Uninstalls only the daemon
uninstall-daemon:
    rm -f {{bin-dst}}
    rm -f {{cosmic-bg-link}}
    rm -rf {{shaders-dir}}
    rm -f {{switch-script-dst}}
    @just data/uninstall
    @just data/icons/uninstall

# Uninstalls only the settings app
uninstall-settings:
    rm -f {{settings-bin-dst}}
    rm -f {{settings-desktop-dst}}
    rm -f {{settings-icon-dst}}
    rm -f {{settings-symbolic-dst}}

# Enable GlowBerry as the cosmic-bg replacement
enable-glowberry:
    #!/usr/bin/env bash
    set -e

    echo "Setting up GlowBerry as cosmic-bg replacement..."

    LOCAL_BIN="$HOME/.local/bin"

    # Check PATH order - ~/.local/bin should come before /usr/bin
    if ! echo "$PATH" | tr ':' '\n' | grep -q "^$LOCAL_BIN$"; then
        echo "WARNING: ~/.local/bin is not in your PATH!"
        echo "         GlowBerry override may not work correctly."
        echo '         Consider adding: export PATH="$HOME/.local/bin:$PATH"'
        echo ""
    fi

    # Create symlink to glowberry
    ln -sf {{bin-dst}} {{cosmic-bg-link}}
    echo "Created symlink: {{cosmic-bg-link}} -> {{bin-dst}}"

    echo ""
    echo "GlowBerry is now active!"
    echo "The original cosmic-bg at /usr/bin/cosmic-bg is unchanged."
    echo ""
    echo "To switch back to original cosmic-bg, run: just disable-glowberry"

# Disable GlowBerry and restore original cosmic-bg
disable-glowberry:
    #!/usr/bin/env bash
    set -e

    echo "Disabling GlowBerry override..."

    if [ -L "{{cosmic-bg-link}}" ]; then
        rm "{{cosmic-bg-link}}"
        echo "Removed {{cosmic-bg-link}} symlink"
        echo ""
        echo "Original cosmic-bg at /usr/bin/cosmic-bg is now active!"
    else
        echo "No GlowBerry override found at {{cosmic-bg-link}}"
    fi

# Check which cosmic-bg is currently active
which-cosmic-bg:
    #!/usr/bin/env bash
    echo "=== cosmic-bg status ==="
    echo ""

    LOCAL_BIN="$HOME/.local/bin"

    # Check PATH order
    echo "PATH order check:"
    if echo "$PATH" | tr ':' '\n' | grep -q "^$LOCAL_BIN$"; then
        echo "  OK: ~/.local/bin is in PATH"
    else
        echo "  WARNING: ~/.local/bin is not in PATH"
    fi

    # Check which binary is in PATH
    echo ""
    WHICH_BG=$(which cosmic-bg 2>/dev/null || echo "not found")
    echo "Active cosmic-bg: $WHICH_BG"

    # Check if it's a symlink
    if [ -L "$WHICH_BG" ]; then
        TARGET=$(readlink -f "$WHICH_BG")
        echo "  -> Points to: $TARGET"
    fi

    # Check ~/.local/bin override
    echo ""
    echo "{{cosmic-bg-link}}:"
    if [ -e "{{cosmic-bg-link}}" ]; then
        ls -la "{{cosmic-bg-link}}"
        if [ -L "{{cosmic-bg-link}}" ]; then
            echo "  -> $(readlink "{{cosmic-bg-link}}")"
        fi
    else
        echo "  Not present (GlowBerry override not active)"
    fi

    # Check /usr/bin/cosmic-bg
    echo ""
    echo "/usr/bin/cosmic-bg:"
    if [ -e /usr/bin/cosmic-bg ]; then
        ls -la /usr/bin/cosmic-bg
    else
        echo "  Not found"
    fi



# Uninstall legacy system-wide installation (requires sudo)
uninstall-legacy:
    #!/usr/bin/env bash
    set -e

    echo "=========================================="
    echo "  Uninstalling legacy system-wide GlowBerry"
    echo "=========================================="
    echo ""
    echo "This removes files from the old system-wide installation"
    echo "that used /usr/ prefix. Requires sudo."
    echo ""

    # First disable the old symlink if present
    if [ -L /usr/local/bin/cosmic-bg ]; then
        TARGET=$(readlink /usr/local/bin/cosmic-bg)
        if echo "$TARGET" | grep -q glowberry; then
            echo "Removing legacy symlink: /usr/local/bin/cosmic-bg -> $TARGET"
            sudo rm -f /usr/local/bin/cosmic-bg
        fi
    fi

    # Binaries
    echo "Removing legacy binaries..."
    sudo rm -f /usr/bin/glowberry
    sudo rm -f /usr/bin/glowberry-settings
    sudo rm -f /usr/bin/glowberry-switch

    # Shaders
    echo "Removing legacy shaders..."
    sudo rm -rf /usr/share/glowberry

    # Desktop files
    echo "Removing legacy desktop files..."
    sudo rm -f /usr/share/applications/io.github.hojjatabdollahi.glowberry.desktop
    sudo rm -f /usr/share/applications/io.github.hojjatabdollahi.glowberry-settings.desktop

    # Metainfo
    sudo rm -f /usr/share/metainfo/io.github.hojjatabdollahi.glowberry.metainfo.xml

    # Icons
    echo "Removing legacy icons..."
    sudo rm -f /usr/share/icons/hicolor/scalable/apps/io.github.hojjatabdollahi.glowberry.svg
    sudo rm -f /usr/share/icons/hicolor/symbolic/apps/io.github.hojjatabdollahi.glowberry-symbolic.svg
    sudo rm -f /usr/share/icons/hicolor/scalable/apps/io.github.hojjatabdollahi.glowberry-settings.svg
    sudo rm -f /usr/share/icons/hicolor/symbolic/apps/io.github.hojjatabdollahi.glowberry-settings-symbolic.svg

    # COSMIC config schema
    echo "Removing legacy COSMIC schema..."
    sudo rm -rf /usr/share/cosmic/io.github.hojjatabdollahi.glowberry

    echo ""
    echo "=========================================="
    echo "  Legacy installation removed!"
    echo "=========================================="
    echo ""
    echo "If you haven't already, disable the legacy override first:"
    echo "  scripts/disable-glowberry-legacy.sh"
    echo ""
    echo "To install the new local version, run:"
    echo "  just install"
    echo ""

# Update desktop database and icon cache after installation
update-cache:
    #!/usr/bin/env bash
    if command -v update-desktop-database &> /dev/null; then
        update-desktop-database {{base-dir}}/share/applications
    fi
    if command -v gtk-update-icon-cache &> /dev/null; then
        gtk-update-icon-cache -f {{base-dir}}/share/icons/hicolor
    fi
    echo "Application cache updated"

# Vendor dependencies locally
vendor:
    mkdir -p .cargo
    cargo vendor --sync Cargo.toml --sync config/Cargo.toml --sync apps/glowberry-settings/Cargo.toml | head -n -1 > .cargo/config.toml
    echo 'directory = "vendor"' >> .cargo/config.toml
    tar pcf vendor.tar vendor
    rm -rf vendor

# Extracts vendored dependencies
vendor-extract:
    #!/usr/bin/env sh
    rm -rf vendor
    tar pxf vendor.tar
