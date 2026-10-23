# wl-cast

Screen capture, encoding and input for Wayland compositors, as a Rust library.

Status: early development. Screen capture, H.264 encoding, RTP streaming and mouse input work on Hyprland.

## Goal

One library that apps and compositors can use to:

- capture the screen
- encode it to H.264
- stream it to another device
- inject keyboard and mouse input

## Compositor support

| Compositor | Status |
|---|---|
| Hyprland | Capture, encoding, streaming and mouse work |
| Sway and other wlroots-based | Planned |
| GNOME, KDE (via xdg-desktop-portal) | Planned |

## Build

```sh
cargo build
cargo test
```

GPU encoding is optional. It needs the FFmpeg development libraries and clang:

```sh
cargo build --features ffmpeg
```

With the feature on, wl-cast tries VA-API (Intel, AMD), then NVENC (Nvidia), then falls back to software (OpenH264). Without it, it always uses software.

## License

You can use wl-cast under either the MIT or the Apache-2.0 license, whichever you prefer. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
