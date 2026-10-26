# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- With the `ffmpeg` feature, software encoding uses x264 when the system FFmpeg has it, before falling back to OpenH264. x264 is several times faster. Its speed setting follows the pixel rate.
- The `WL_CAST_ENCODER` environment variable forces an encoder: `vaapi`, `nvenc`, `x264` or `openh264`. A forced encoder that cannot open is an error, not a silent fallback.

## [0.2.3] - 2026-10-25

### Added

- `Capture::by_name` captures the monitor with that compositor name, such as `eDP-1`. `Capture::new` still uses an index.
- `WfdSource` runs the Miracast RTSP setup. `MpegTsRtp` packs H.264 into MPEG-TS RTP. `scale_bgra` resizes a frame to the mode the sink accepted.

### Fixed

- The sink sends `PLAY`. Answering that starts the video. Sending `PLAY` to the sink is rejected.

## [0.2.2] - 2026-10-25

### Added

- The `portal` feature captures through xdg-desktop-portal and PipeWire on compositors without ext-image-copy-capture or wlr-screencopy, such as GNOME and KDE. `WL_CAST_CAPTURE=portal` forces it. Failures are reported as `Error::Portal`.
- The same feature controls the mouse and keyboard through the remote desktop portal when the compositor has no virtual pointer or keyboard. `WL_CAST_INPUT=portal` forces it. Key codes are Linux key codes, the same as the virtual keyboard.

### Fixed

- An odd width or height is encoded by duplicating the last column or row, instead of crashing inside OpenH264.

## [0.2.1] - 2026-10-25

### Added

- `EncoderConfig` and `Encoder::with_config` set the bitrate, frame rate and H.264 profile. `Encoder::new` still uses 8 Mbit/s and 60 frames per second.
- `SessionConfig.encoder` passes those settings into a WebRTC session.
- `StopHandle` stops a WebRTC session from another thread. `RemoteSession::take_events` reports when a viewer connects or disconnects. `Capture::set_stop_handle` unblocks `next_frame` while the screen is idle, including after the viewer leaves.
- The WebRTC example has a Keyboard button. On a phone it opens the phone keyboard and shows Esc, modifiers and arrow keys.

## [0.2.0] - 2026-10-25

### Changed

- Breaking: `Error`, `Frame` and `SessionConfig` are `#[non_exhaustive]`. Build a frame with `Frame::new` and session settings with `SessionConfig::new`.
- Breaking: `Input::button` and `Input::key` take a `PressState` instead of a bool.
- Breaking: `Error::Connect` and `Error::Dispatch` carry the reason as text. They no longer expose `wayland-client` types.

### Added

- CI runs capture and input against a headless Sway.

### Fixed

- Capture keeps going when a monitor is resized or rotated, and returns frames of the new size. A WebRTC session opens a new encoder for that size.

## [0.1.2] - 2026-10-24

### Added

- `Debug` on every public type. `AccessToken` prints `AccessToken(..)` and never the secret. `Frame` prints the data length instead of the pixels.
- `Hash` on `MouseButton`, `Clone` on `Frame`, and `Clone`, `PartialEq` and `Eq` on `SessionConfig`.

### Changed

- Every public function that returns `Result` documents its errors.

## [0.1.1] - 2026-10-24

### Added

- `Capture` falls back to wlr-screencopy when the compositor lacks ext-image-copy-capture. This adds niri.
- The `WL_CAST_CAPTURE` environment variable forces a capture backend: `ext` or `wlr`.
- Tested on niri, Sway, labwc and river, in addition to Hyprland.

### Fixed

- Frames from rotated or flipped monitors now come out the right way up. `Capture::size` returns the size after rotation.
- `Capture` accepts Xbgr8888 and Abgr8888 buffers, which some wlroots compositors such as Sway offer instead of Xrgb8888.

### Changed

- Documented the whole public API, with `# Errors` sections, and added crate docs with a compile-checked example.
- Added the `repository` field to `Cargo.toml`.

## [0.1.0] - 2026-10-24

### Added

- Project setup: crate, license, README, CI.
- `Capture`: capture frames from a monitor using ext-image-copy-capture.
- `Encoder`: encode frames to H.264 in software with OpenH264.
- `RtpSender`: stream H.264 over RTP/UDP, with an SDP file for players.
- `Frame::captured_at`: capture time, used for playback timing.
- `Input`: move, click and scroll the mouse with wlr-virtual-pointer.
- `Input::key`: press and release keys with virtual-keyboard, using the compositor's own keymap.
- `webrtc` feature: `RemoteSession` streams the screen to a browser over WebRTC and applies its mouse and keyboard input. Supports a fixed UDP port and an advertised public IP for direct internet connections.
- `AccessToken`: random secret a viewer must present to connect.
- `Encoder::request_keyframe`: make the next frame a full picture for a new viewer.
- `ffmpeg` feature: GPU encoding with VA-API (Intel, AMD) or NVENC (Nvidia), falling back to OpenH264.

### Changed

- `Encoder::new` now takes the frame width and height. Use `Capture::size`.
- The minimum supported Rust version is 1.89.
- `RemoteSession::run` returns an error if the viewer does not connect within 30 seconds.
- `Encoder` can be moved to another thread with the `ffmpeg` feature, as it can without it.
- `Encoder::encode` returns `Error::InvalidFrame` when the frame size or data length does not match the encoder.
