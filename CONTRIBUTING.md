# Contributing

Thanks for helping. wl-cast is a small Rust library, so keep changes small and focused.

## Setup

With Nix, run `nix develop` in the repository. It provides everything below.

Without Nix you need Rust 1.89 or newer. The optional features need more:

- `ffmpeg`: the FFmpeg development libraries, clang and pkg-config.
- `webrtc`: a C compiler.

## Build and test

```sh
cargo build
cargo test
cargo test --features ffmpeg,webrtc
```

CI has no Wayland session. Code that talks to a compositor (`Capture`, `Input`) is tested by hand with the programs in `examples`.

## Before you open a pull request

```sh
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --features ffmpeg,webrtc -- -D warnings
cargo test
```

## Code style

- No `unwrap`, `expect` or `panic!` in library code. Return `wl_cast::Error`.
- Keep `unsafe` small.
- Every logic change needs a test.
- Hardware paths (VA-API, NVENC, DMA-BUF) must be optional and fall back to something that works everywhere.
- Code for one specific compositor stays in its own backend file.

## Commits

Use conventional commits: `feat:`, `fix:`, `docs:`, `chore:`, `refactor:`, `test:`, `perf:`. Keep the subject short.

## Pull requests

- One change per pull request.
- Add a line to `CHANGELOG.md` under `Unreleased`.
- Say which compositor, GPU and distro you tested on.

## New compositor backends

Open an issue first so we can agree on the approach.
