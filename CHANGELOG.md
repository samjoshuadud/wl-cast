# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- `EncoderConfig` and `Encoder::with_config` set the bitrate, frame rate and H.264 profile. `Encoder::new` still uses 8 Mbit/s and 60 frames per second.
- `SessionConfig.encoder` passes those settings into a WebRTC session.

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
