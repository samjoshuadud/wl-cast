# wl-cast

Screen capture, encoding and input for Wayland compositors, as a Rust library.

Status: early development. Screen capture, H.264 encoding, RTP streaming, mouse and keyboard input work on Hyprland.

## Goal

One library that apps and compositors can use to:

- capture the screen
- encode it to H.264
- stream it to another device
- inject keyboard and mouse input

## Compositor support

| Compositor | Status |
|---|---|
| Hyprland | Capture, encoding, streaming, mouse and keyboard work |
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

## Remote viewer (WebRTC)

With the `webrtc` feature, `RemoteSession` streams the screen to a browser and applies the viewer's mouse and keyboard. The app handles the web page and login. See `examples/webrtc.rs`.

```sh
cargo run --release --features ffmpeg,webrtc --example webrtc
```

It prints a link with a secret token. Open it in a browser on the same network.

### From the internet

wl-cast connects directly. There is no relay server. Either:

- forward TCP and UDP port 8080 on your router and pass your public IP: `cargo run --release --features webrtc --example webrtc -- 203.0.113.7`
- or use a VPN such as Tailscale.

This does not work behind carrier-grade NAT without a VPN. The example page uses plain HTTP, so on untrusted networks use a VPN or put it behind an HTTPS reverse proxy.

### Viewer messages

The viewer sends one text line per event on a data channel named `input`.

| Message | Meaning |
|---|---|
| `m X Y` | Move the pointer. X and Y are fractions of the screen, 0 to 1. |
| `b N S` | Mouse button N (0 left, 1 middle, 2 right). S is 1 for down, 0 for up. |
| `w DX DY` | Scroll by pixels. |
| `k CODE S` | Key with Linux key code CODE. S is 1 for down, 0 for up. |

## License

You can use wl-cast under either the MIT or the Apache-2.0 license, whichever you prefer. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
