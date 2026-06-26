<p align="center">
  <img src="data/GlowBerry.svg" alt="GlowBerry Logo" width="128">
</p>

# GlowBerry

An enhanced background/wallpaper service with live shader support for COSMIC DE.

Disclaimer: This project extends the functionality of cosmic-bg with live shader wallpapers. When set up correctly, cosmic-session will run GlowBerry instead of cosmic-bg.

https://github.com/user-attachments/assets/44c1a6a2-4c13-4d02-b108-48284f1a5def

Multi monitor support:
<img width="1523" height="987" alt="image" src="https://github.com/user-attachments/assets/1662eae0-4b26-4338-9c72-970c2de5ef91" />


## Features

- Live GPU-rendered shader wallpapers (WGSL)
- Static image wallpapers with multi monitor support
- Per-display configuration
- Power saving options (pause/reduce FPS on battery)
- Settings application for easy configuration

## Installation

Build and install with [just](https://github.com/casey/just):

```sh
just
just install
```

This installs GlowBerry to `~/.local/bin/glowberry` and creates a symlink at `~/.local/bin/cosmic-bg` pointing to it. No sudo required.

### Dependencies

- just
- cargo / rustc (install from https://rustup.rs/)
- libwayland-dev
- libxkbcommon-dev
- mold
- pkg-config

## Enabling GlowBerry

GlowBerry works by intercepting cosmic-session's call to `cosmic-bg`. The installer creates a symlink at `~/.local/bin/cosmic-bg` that points to `~/.local/bin/glowberry`. Since `~/.local/bin` is searched before `/usr/bin` in PATH, cosmic-session will run GlowBerry instead.

> [!IMPORTANT]
> For this to work, `~/.local/bin` must appear before `/usr/bin` in your PATH. You can verify this by running:
> ```sh
> echo $PATH | tr ':' '\n' | grep -n bin
> ```

### Using the switch script

Enable GlowBerry (you may need to restart for this to take effect):
```sh
glowberry-switch enable
```

Disable GlowBerry (restore original cosmic-bg):
```sh
glowberry-switch disable
```

Check current status:
```sh
glowberry-switch status
```

### Using the settings app

You can also enable/disable GlowBerry from the settings application (`glowberry-settings`). Open the settings drawer and toggle "Use GlowBerry as default". You may need to restart to clean up old cosmic-bg and use GlowBerry properly.

### Manual setup

If you prefer to set it up manually:

```sh
# Enable GlowBerry
ln -sf ~/.local/bin/glowberry ~/.local/bin/cosmic-bg
pkill cosmic-bg  # Restart the service

# Disable GlowBerry
rm ~/.local/bin/cosmic-bg
pkill glowberry  # Restart the service
```

## Adding Shaders

Shader wallpapers are WGSL files. GlowBerry searches for shaders in XDG data directories:
- `~/.local/share/glowberry/shaders/` (user-local, installed by default)
- Directories listed in `$XDG_DATA_DIRS` (e.g. `/usr/share/glowberry/shaders/`)

Example shaders are included in the `examples/` directory and installed automatically by `just install`.

To install additional shaders manually:
```sh
cp my_shader.wgsl ~/.local/share/glowberry/shaders/
```

## Uninstall

```sh
just uninstall
```

### Removing a legacy system-wide installation

If you previously installed GlowBerry system-wide (with `sudo just install` to `/usr/`), first disable the old override, then remove the legacy files:

```sh
scripts/disable-glowberry-legacy.sh
sudo just uninstall-legacy
```

## Why GlowBerry?

With the right shader, your desktop can be a glowing berry.
