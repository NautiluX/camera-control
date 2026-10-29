# Camera Control

A Linux camera-control app built with Rust and Iced 0.14. No OBSBOT SDK is required for its standard camera controls.

## Features

- Camera selection using stable USB device identities, with automatic reconnection checks.
- White-balance presets: Tungsten (3200 K), Fluorescent (4200 K), Daylight (5500 K), Cloudy (6500 K), and Shade (7500 K). These are convenient approximate color-temperature settings, not manufacturer-specific scene modes. The existing slider remains available for fine-tuning.
- Automatic white balance, plus brightness, contrast and saturation where supported.
- Optional live preview with camera-supported resolution choices and a frame-rate selector up to 60 fps, plus a preview-only mirror toggle. Choices persist per camera. Enable it to inspect color changes; disable it to release capture for Meet or another app. Preview stops when hiding to the tray or changing cameras, and starts off when launching the app.
- Optional settings watcher: checks every 500 ms and restores your chosen color settings. Settings are saved per camera.
- Tray icon with Show, Hide and Quit. Closing the window hides it while the watcher continues. If the desktop has no working tray, the app keeps a visible window and closing it quits.
- Configurable **Start at login**. Startup launches the tray app without a window; opening the app again shows the existing instance.

Camera controls remain editable while watching. Unsupported controls are omitted. Preview uses native V4L2 capture, with MJPEG or YUYV decoding on a background thread, and keeps only the latest frame. Capture requests the chosen resolution and frame rate from the camera; the status line shows the requested rate, negotiated camera rate, and measured decoding rate. Rates up to 60 fps can be requested even when mode enumeration is incomplete. If a request is rejected, the app falls back to an advertised rate and reports what it actually achieved. A 60 fps selection cannot make a camera mode that is limited to 30 fps produce 60 distinct frames. There is no fixed 10 fps cap. Mirroring flips only the local preview and leaves the camera orientation used by other apps unchanged. Changing resolution or frame rate restarts capture; changing mirroring does not. A camera may need a second or two to wake or release capture.

## Build and run

Requires Linux x86_64 or aarch64, Rust 1.93 or newer, the usual Wayland/X11 desktop libraries, and libclang for the V4L2 capture bindings during compilation. A StatusNotifierItem tray host is required for the tray icon (e.g. KDE, a supporting GNOME extension, or a compatible panel).

```sh
cargo build --release
python3 install.py target/release/camera-control --autostart
cargo run
cargo run -- --list
cargo run -- --background
cargo test
```

`--list` reads camera information without changing settings. `--background` starts the app in the tray. Settings are stored at `$XDG_CONFIG_HOME/camera-control/settings.json` (normally `~/.config/camera-control/settings.json`). Watching is off by default for a new camera.

The local installer adds an application-menu launcher and installs the binary in `~/.local/share/camera-control-iced/` (respecting `XDG_DATA_HOME`). Its `--autostart` option enables startup.

Put the executable in a stable installation location before enabling **Start at login**: the app registers the path of the running executable. The desktop autostart entry is `$XDG_CONFIG_HOME/autostart/camera-control-iced.desktop`. Uncheck **Start at login** to remove it. Use **Quit** in the tray menu or window footer to stop the app completely.

The optional standalone `--watch` mode enforces all saved enabled profiles without a UI or tray; stop it with Ctrl+C. Do not run that mode alongside the tray app or another color-enforcement utility for the same camera.

## Checks

```sh
cargo test
cargo clippy --all-targets -- -D warnings
```

An additional ignored hardware test starts capture, receives a frame, releases it and repeats. Run only when the chosen camera is free for capture:

```sh
CAMERA_CONTROL_TEST_CAMERA=/dev/v4l/by-id/YOUR_CAMERA-video-index0 \
  cargo test hardware_preview -- --ignored --nocapture
```

## SDK integration

No vendor headers or binaries are included. Keep any future OBSBOT SDK outside the repository. A future adapter can accept a local SDK path after its implementation and camera support are verified. Gestures and other OBSBOT-specific functions are not implemented yet.

- `src/camera.rs`: standard V4L2 color controls.
- `src/preview.rs`: optional capture and frame decoding.
- `src/worker.rs`: camera I/O and settings watcher.
- `src/settings.rs`: saved profiles.
- `src/desktop.rs`: tray, single-instance handling and startup.
- `src/ui.rs`: Iced interface and visual styles.
- `src/main.rs`: application state and lifecycle.

## Design references

Preset temperatures are approximate starting points. Daylight uses 5500 K. Camera manufacturers use somewhat different values; see [Nikon's white-balance explanation](https://www.nikonusa.com/learn-and-explore/c/tips-and-techniques/setting-white-balance) and [OBSBOT's Lite Mode overview](https://www.obsbot.com/news/all-new-lite-mode-for-obsbot-center) for the lighting concepts and grouping of common controls.

## Dependency maintenance

GitHub Actions checks formatting, lint, tests, the application build, and installer
syntax on every pull request and push to `main`. Camera hardware is not needed;
the explicit hardware test remains opt-in.

Dependabot checks Rust dependencies and GitHub Actions weekly, with compatible
updates grouped to reduce PR noise. Patch and minor updates merge automatically
only after the required **Rust checks** succeeds. Major updates and failing updates
remain open for review. Security update PRs follow the same merge policy.

## Releases

[Download the latest release](https://github.com/NautiluX/camera-control/releases/latest).
The Linux x86_64 archive includes the executable, installer and license notices;
it requires glibc 2.39 or newer (Ubuntu 24.04 or equivalent).

The Release workflow runs every Monday at 09:00 UTC and publishes automatically
when `main` has changed since the previous release. It can also be run manually
from GitHub Actions. Every release passes formatting, lint and tests before
publication. The first version uses the Cargo package version; subsequent releases
increment the patch version unless main specifies a higher version. Version
stamping is committed only to the release tag, leaving main unchanged. No release
is created when the source tree is unchanged. SHA-256 checksums accompany downloads.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
