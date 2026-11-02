# Contributing to RiffBoard

Thanks for helping! A few things to know first.

- **Security comes first.** RiffBoard runs on the public internet. Any new
  server route must be added to the allowlist in `src/guard.rs` with
  tests, and must not let a visitor change server state beyond starting a run
  and posting a score. Read [CLAUDE.md](CLAUDE.md) for the rules.
- **Branches:** open pull requests against `development`. `main` only gets
  tested releases.
- **Tests:** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test` and `node --test tests/js/*.test.js` must pass.
- **Licence:** contributions are accepted under the project's licence
  (AGPL-3.0-only). Third-party code must be AGPL-compatible.
- **Security issues:** please report them privately, see [SECURITY.md](SECURITY.md).

Changes that belong in the player engine, the highway renderers or the
note-detection plugin are often better sent upstream to
[fee[dB]ack](https://github.com/got-feedback/feedBack) first.
