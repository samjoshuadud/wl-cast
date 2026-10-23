# wl-cast

Screen capture, encoding and input for Wayland compositors, as a Rust library.

Status: early development. Nothing works yet.

## Goal

One library that apps and compositors can use to:

- capture the screen
- encode it to H.264
- stream it to another device
- inject keyboard and mouse input

## Compositor support

| Compositor | Status |
|---|---|
| Hyprland | Planned (first target) |
| Sway and other wlroots-based | Planned |
| GNOME, KDE (via xdg-desktop-portal) | Planned |

## Build

```sh
cargo build
cargo test
```

## License

You can use wl-cast under either the MIT or the Apache-2.0 license, whichever you prefer. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
