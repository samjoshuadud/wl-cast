# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

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
