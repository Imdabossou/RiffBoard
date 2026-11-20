# Contributing to RiffBoard

Thanks for helping! A few things to know first.

- **Security comes first.** RiffBoard runs on the public internet. Any new
  server route must be added to the allowlist in `src/guard.rs` with
  tests, and must not let a visitor change server state beyond starting a run
  and posting a score. Read [AGENTS.md](AGENTS.md) for the rules.
- **Branches:** open pull requests against `development`. `main` only gets
  tested releases.
- **Tests:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  the wasm clippy and `cargo test --workspace` must pass, and `scripts/build-game.sh`
  must build (AGENTS.md lists the commands).
- **Smoothness:** the game's frame loop must not allocate or touch the DOM.
  AGENTS.md explains the rules.
- **Licence:** contributions are accepted under the project's licence
  (AGPL-3.0-only). Third-party code must be AGPL-compatible.
- **Security issues:** please report them privately, see [SECURITY.md](SECURITY.md).

Changes to the feedpak song format belong upstream, in
[fee[dB]ack](https://github.com/got-feedback/feedBack)'s format spec.
