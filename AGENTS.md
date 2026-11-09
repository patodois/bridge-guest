# Agent guide

bridge-guest renders rewrite-bridge provider models inside a passthrough host game for developers integrating cross-game assets.
It is a GPU guest built with Bevy and rewrite-bridge, using the host's camera and a colour/depth pixel file.

## Toolchain, build and CI

- CI uses stable Rust on Linux and macOS; no minimum Rust version is declared.
- Default features are `wt`, `wf`, `hytale`, `nocturne` and `mc`.
- On Linux, CI installs `libwayland-dev libxkbcommon-dev libx11-dev libudev-dev libasound2-dev`.
- Exact CI checks from `.github/workflows/ci.yml`:

```sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
```

The README's executable build is `cargo build --release`; CI compiles through Clippy and tests.
To select one provider, the README uses `--no-default-features --features hytale`.

## Code map

- `src/main.rs`, `cli.rs`, `providers.rs`: application, arguments and provider selection.
- `src/camera.rs`, `placement.rs`, `bounds.rs`: host camera and model placement.
- `src/render.rs`, `depth.rs`, `linear_depth.wgsl`: GPU colour and linear-depth rendering.
- `src/frames.rs`, `stats.rs`: frame assembly and session measurements.
- `src/selftest.rs`, `examples/`: generated test geometry and host/depth probes.

## Project rules

- Read assets from the user's own game installation; no game data is bundled or cached.
- Provider dependencies are public Git revisions; keep one copy of rewrite-bridge 0.4 in the build.
- Game-data tests skip when their required installation is absent.
- Keep at most two renders in flight; the newest camera wins when both are busy.
- Only perspective host projections are supported; colour and depth must describe the same frame.
- Endpoint files are re-read on reconnect, their access is checked, and tokens are never printed.
- Pixel files live in the temporary directory and are removed when the session ends.

## Public files

- Never commit game files, extracted assets, renders, decompiled code or secrets.
- The public-tree gate needs Python 3.9+ and Git; run it from the repository root:
  `python3 tools/check_public_tree.py` (checks Git's index, including staged new files).
- CI checks committed changes with `python3 tools/check_public_tree.py --range BASE..HEAD`.

## Contributing, license and AI

- Open an issue to discuss a problem or send a pull request with the change.
- Before a pull request, run the applicable CI commands above and the public-tree gate.
- License: MIT OR Apache-2.0, at your option; see `LICENSE-MIT` and `LICENSE-APACHE`.

The code was written with AI coding agents (Anthropic's Claude and OpenAI's GPT models) under the maintainer's direction.
AI-assisted contributions are welcome when their use is disclosed and the changes are reviewed.
