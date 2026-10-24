# wl-cast

Screen capture, encoding and input for Wayland compositors, as a Rust library.

Status: early. Tested on Hyprland, niri, Sway, labwc and river. The API can still change in 0.x releases.

## Goal

One library that apps and compositors can use to:

- capture the screen
- encode it to H.264
- stream it to another device
- inject keyboard and mouse input

## Quick example

```rust
use wl_cast::{Capture, Encoder};

let mut capture = Capture::new(0)?;
let (width, height) = capture.size();
let mut encoder = Encoder::new(width, height)?;

let frame = capture.next_frame()?;
let h264 = encoder.encode(&frame)?;
```

`Input` moves the mouse and presses keys. `RtpSender` and `RemoteSession` send the video out. The `examples` folder has a runnable demo for each.

`Encoder::new` uses 8 Mbit/s and 60 frames per second. `Encoder::with_config` changes the bitrate, frame rate and H.264 profile. A WebRTC session reads the same settings from `SessionConfig.encoder`.

## Compositor support

wl-cast picks a backend by what the compositor offers, not by its name. Capture uses ext-image-copy-capture when available, otherwise wlr-screencopy. Input uses the wlr virtual pointer and the virtual keyboard.

| Compositor | Tested version | Capture | Input | Status |
|---|---|---|---|---|
| Hyprland | 0.56.2 | ext, wlr | yes | Works |
| niri | 26.04 | wlr | yes | Works |
| Sway | 1.12 | ext, wlr | yes | Works |
| labwc | 0.20.2 | ext, wlr | yes | Works |
| river | 0.3.17 (river-classic) | ext, wlr | yes | Works |
| COSMIC | - | ext | keyboard only | Not tested. Mouse needs the portal backend. |
| GNOME, KDE | - | - | - | Planned, through xdg-desktop-portal |

Hyprland was tested directly. The others were tested running nested inside Hyprland.

To force a capture backend, set `WL_CAST_CAPTURE=ext` or `WL_CAST_CAPTURE=wlr`.

## Build

Needs Rust 1.89 or newer.

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

It prints a link with a secret token. Open it in a browser on the same network. `RemoteSession::handle` stops the session from another thread, and `take_events` reports when the viewer connects or leaves.

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

## Known limits

- Linux and Wayland only. GNOME and KDE are not supported yet.
- One viewer at a time, and the session ends when the viewer leaves or does not connect within 30 seconds.
- Unplugging a monitor during a session is not handled. A resize or rotation is followed.
- No audio, clipboard or multi-monitor switching.
- GPU encoding with NVENC (Nvidia) is untested. VA-API (Intel, AMD) is tested on AMD.
- Software encoding is slow on busy full-screen video (about 10 frames per second at 1080p).
- Internet access is direct only: port forwarding or a VPN. There is no relay server.
- The example web page uses plain HTTP. Do not expose it on untrusted networks without a VPN or an HTTPS proxy.
- The example web server is for demos. One slow client can stall it.
- The software encoder is OpenH264 built from source. Cisco's patent license covers only their prebuilt binaries, so check the H.264 licensing before you distribute binaries.

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md). To report a vulnerability, see [SECURITY.md](SECURITY.md).

## License

You can use wl-cast under either the MIT or the Apache-2.0 license, whichever you prefer. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
