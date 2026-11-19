#!/usr/bin/env bash
# Builds the browser game (crates/game) into static/game/pkg/.
#
# Needs: the wasm32-unknown-unknown Rust target, wasm-bindgen-cli at exactly
# the version pinned in crates/game/Cargo.toml, and (optional) binaryen's
# wasm-opt. The output is not committed; the Docker build runs this too.
set -euo pipefail
cd "$(dirname "$0")/.."
out=static/game/pkg
want=$(sed -n 's/^wasm-bindgen = "=\(.*\)"/\1/p' crates/game/Cargo.toml)
have=$(wasm-bindgen --version | awk '{print $2}')
if [ "$want" != "$have" ]; then
    echo "wasm-bindgen CLI is $have but crates/game pins $want" >&2
    exit 1
fi
cargo build -p riffboard-game --target wasm32-unknown-unknown --profile wasm --locked
rm -rf "$out"
wasm-bindgen --target web --no-typescript --out-dir "$out" \
    target/wasm32-unknown-unknown/wasm/riffboard_game.wasm
if command -v wasm-opt >/dev/null; then
    wasm-opt -O3 --enable-bulk-memory --enable-nontrapping-float-to-int \
        "$out/riffboard_game_bg.wasm" -o "$out/riffboard_game_bg.wasm"
fi
ls -l "$out"
