# Building RiffBoard

## Requirements

You'll need Rust (stable, edition 2024) with the `wasm32-unknown-unknown` target, the [wasm-bindgen CLI](https://github.com/wasm-bindgen/wasm-bindgen) at the version `crates/game/Cargo.toml` pins, and optionally binaryen's `wasm-opt`.

## Commands

```bash
bash scripts/build-game.sh                    # builds the game into static/game/pkg/
DLC_DIR=./library CONFIG_DIR=./.config cargo run      # serves http://localhost:8000

cargo test --workspace                        # unit tests + end-to-end tests against the real server
cargo clippy -p riffboard-game --target wasm32-unknown-unknown -- -D warnings
```

Before a pull request, run the full list of checks in [CONTRIBUTING.md](CONTRIBUTING.md).

## Layout

The project is a Cargo workspace:

| Part | What it is |
|---|---|
| `src/` | The server (axum + tokio, SQLite only for scores). |
| `crates/core` | Game rules shared by the server and the game: chart model, judging, scoring, leaderboard checks, the note detector. Plain Rust, tested natively. |
| `crates/game` | The browser game: WebGL2 renderer, highway layout, camera, HUD. Compiled to wasm. |
| `static/game/` | The page around it: menus in plain HTML, Web Audio playback and the song clock (`player.js`), and the microphone worklet. No build step. |

To check the detector against a recording, run `cargo run -p riffboard-core --example detect_bench --release -- chart.json song.f32` (see the file for details; `chart.json` is what `/api/chart/<song>` returns). [AGENTS.md](AGENTS.md) has a fuller map and the rules.

## History

RiffBoard started as a fork of [fee[dB]ack](https://github.com/got-feedback/feedBack), a self-hosted practice app designed for one trusted user on a home network. RiffBoard rewrote the server in Rust for a public site, and from 0.4 the player is its own game engine: Rust compiled to WebAssembly, drawing with WebGL2. Upstream's notes are kept in [docs/upstream/](docs/upstream/README.md).
