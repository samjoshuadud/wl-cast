# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Project setup: crate, license, README, CI.
- `Capture`: capture frames from a monitor using ext-image-copy-capture.
- `Encoder`: encode frames to H.264 in software with OpenH264.
- `RtpSender`: stream H.264 over RTP/UDP, with an SDP file for players.
- `Frame::captured_at`: capture time, used for playback timing.
- `Input`: move, click and scroll the mouse with wlr-virtual-pointer.
- `Input::key`: press and release keys with virtual-keyboard, using the compositor's own keymap.
- `Encoder::request_keyframe`: make the next frame a full picture for a new viewer.
- `ffmpeg` feature: GPU encoding with VA-API (Intel, AMD) or NVENC (Nvidia), falling back to OpenH264.

### Changed

- `Encoder::new` now takes the frame width and height. Use `Capture::size`.
