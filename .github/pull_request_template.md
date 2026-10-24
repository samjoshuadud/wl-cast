## What changed

Closes #

## How you tested it

Compositor and version:
GPU and driver:
Distro:

## Checklist

- [ ] This pull request makes one change
- [ ] The code follows the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/checklist.html)
- [ ] New public items are documented, with `# Errors`, `# Panics` or `# Safety` sections where they apply
- [ ] Every `unsafe` block has a `// SAFETY:` comment
- [ ] Library code returns `wl_cast::Error` instead of using `unwrap`, `expect` or `panic!`
- [ ] Hardware or compositor specific code is optional and stays in its backend file
- [ ] Tests added or updated for logic changes
- [ ] `cargo fmt`, `cargo clippy --all-targets --features ffmpeg,webrtc -- -D warnings` and `cargo test` pass
- [ ] A line added to `CHANGELOG.md` under `Unreleased`
- [ ] README and docs updated if behavior changed

See [CONTRIBUTING.md](https://github.com/samjoshuadud/wl-cast/blob/main/CONTRIBUTING.md) for the full guidelines.
