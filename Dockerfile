# RiffBoard container image: one static Rust binary plus the browser files.
#
# The binary is linked statically (musl), so the runtime image is
# distroless/static: no shell, no package manager, no libc, no OpenSSL.

FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
# Dependencies first, so source edits don't refetch/rebuild them.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN mkdir src && echo "fn main() {}" > src/main.rs \
    && CARGO_BUILD_JOBS=2 cargo build --release --locked \
    && rm -rf src
COPY src ./src
RUN touch src/main.rs && CARGO_BUILD_JOBS=2 cargo build --release --locked \
    && mkdir -p /out/config

# The browser game: Rust compiled to WebAssembly (scripts/build-game.sh).
FROM rust:1-alpine AS game
RUN apk add --no-cache musl-dev bash binaryen
WORKDIR /src
COPY crates/game/Cargo.toml crates/game/Cargo.toml
# The wasm-bindgen CLI must match the version crates/game pins.
RUN rustup target add wasm32-unknown-unknown \
    && CARGO_BUILD_JOBS=2 cargo install wasm-bindgen-cli --locked \
        --version "$(sed -n 's/^wasm-bindgen = "=\(.*\)"/\1/p' crates/game/Cargo.toml)"
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY scripts/build-game.sh scripts/build-game.sh
RUN mkdir -p src static && echo "fn main() {}" > src/main.rs \
    && CARGO_BUILD_JOBS=2 bash scripts/build-game.sh

# Precompressed copies of the frontend (brotli + gzip). The server sends them
# to browsers that accept them.
FROM alpine:3 AS assets
RUN apk add --no-cache brotli gzip
COPY static /assets/static
COPY --from=game /src/static/game/pkg /assets/static/game/pkg
RUN find /assets -type f -size +1k \( -name '*.js' -o -name '*.mjs' -o -name '*.css' -o -name '*.html' \
        -o -name '*.json' -o -name '*.svg' -o -name '*.wasm' -o -name '*.txt' \) \
        -exec brotli -q 11 -k {} \; -exec gzip -9 -k -n {} \;

FROM gcr.io/distroless/static-debian12:nonroot
WORKDIR /app
COPY --from=build /src/target/release/riffboard /app/riffboard
COPY --from=assets /assets/static /app/static
# An owned, empty /config so a fresh named volume is writable by nonroot.
COPY --from=build --chown=nonroot:nonroot /out/config /config
ENV DLC_DIR=/dlc \
    CONFIG_DIR=/config \
    STATIC_DIR=/app/static \
    HOST=0.0.0.0 \
    PORT=8000
# /config (scores, unpack cache, admin token) is the only place it writes; the
# library at /dlc is only ever read. Runs as the image's nonroot user unless
# the deployment sets another (e.g. the owner of the library files).
USER nonroot
EXPOSE 8000
VOLUME ["/config"]
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["/app/riffboard", "healthcheck"]
ENTRYPOINT ["/app/riffboard"]
