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

### Debian package (Pop!_OS / Ubuntu 24.04)

Download the `.deb` from the [latest release](https://github.com/hojjatabdollahi/glowberry/releases) and install it:

```sh
sudo apt install ./glowberry_*.deb
```

Or add the Cloudsmith repository and install from there:

```sh
curl -1sLf 'https://dl.cloudsmith.io/public/cosmetics/glowberry/setup.deb.sh' | sudo -E bash
sudo apt install glowberry
```

The package installs `glowberry`, `glowberry-settings` and `glowberry-switch` to `/usr/bin`, plus the bundled shaders to `/usr/share/glowberry/shaders/`. It does not touch `/usr/bin/cosmic-bg`; enabling GlowBerry is still per-user (see [Enabling GlowBerry](#enabling-glowberry)):

```sh
glowberry-switch enable
```

To build the package yourself, install [cargo-deb](https://github.com/kornelski/cargo-deb) and run `just deb`. The result lands in `target/debian/`.

#### Switching from a `just install` to the package

Files in `~/.local` shadow the ones in `/usr`, so remove the source install first. From your checkout:

```sh
just uninstall                          # removes ~/.local/bin/glowberry*, shaders, desktop files, defaults
sudo apt install ./glowberry_*.deb
glowberry-switch enable                 # re-points ~/.local/bin/cosmic-bg at /usr/bin/glowberry
```

`just uninstall` only removes the bundled shaders; anything you added to `~/.local/share/glowberry/shaders/` is kept. If you no longer have the checkout, delete these by hand:

```sh
rm -f ~/.local/bin/{glowberry,glowberry-settings,glowberry-switch,cosmic-bg}
rm -f ~/.local/share/applications/io.github.hojjatabdollahi.glowberry{,-settings}.desktop
rm -f ~/.local/share/metainfo/io.github.hojjatabdollahi.glowberry.metainfo.xml
rm -f ~/.local/share/icons/hicolor/{scalable,symbolic}/apps/io.github.hojjatabdollahi.glowberry*
rm -rf ~/.local/share/cosmic/io.github.hojjatabdollahi.glowberry
```

### From source

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

GlowBerry works by intercepting cosmic-session's call to `cosmic-bg`. A symlink at `~/.local/bin/cosmic-bg` points to the `glowberry` binary (`~/.local/bin/glowberry` for a source install, `/usr/bin/glowberry` for the `.deb`). Since `~/.local/bin` is searched before `/usr/bin` in PATH, cosmic-session will run GlowBerry instead. `just install` creates the symlink for you; after installing the `.deb`, run `glowberry-switch enable` once.

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
# Enable GlowBerry (use /usr/bin/glowberry if installed from the .deb)
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

Example shaders are included in the `examples/` directory and installed automatically by `just install` (to `~/.local/share`) and by the `.deb` (to `/usr/share`).

To install additional shaders manually:
```sh
cp my_shader.wgsl ~/.local/share/glowberry/shaders/
```

## Uninstall

Source install:

```sh
just uninstall
```

Debian package (disable the override first so cosmic-bg comes back):

```sh
glowberry-switch disable
sudo apt remove glowberry
```

### Removing a legacy system-wide installation

If you previously installed GlowBerry system-wide (with `sudo just install` to `/usr/`), first disable the old override, then remove the legacy files:

```sh
scripts/disable-glowberry-legacy.sh
sudo just uninstall-legacy
```

## Why GlowBerry?

With the right shader, your desktop can be a glowing berry.
