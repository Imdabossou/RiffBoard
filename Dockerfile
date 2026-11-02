# RiffBoard container image: one static Rust binary plus the browser files.
#
# The binary is linked statically (musl), so the runtime image is
# distroless/static: no shell, no package manager, no libc, no OpenSSL.

FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
# Dependencies first, so source edits don't refetch/rebuild them.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs \
    && CARGO_BUILD_JOBS=2 cargo build --release --locked \
    && rm -rf src
COPY src ./src
RUN touch src/main.rs && CARGO_BUILD_JOBS=2 cargo build --release --locked \
    && mkdir -p /out/config

FROM gcr.io/distroless/static-debian12:nonroot
WORKDIR /app
COPY --from=build /src/target/release/riffboard /app/riffboard
COPY static /app/static
COPY plugins /app/plugins
# An owned, empty /config so a fresh named volume is writable by nonroot.
COPY --from=build --chown=nonroot:nonroot /out/config /config
ENV DLC_DIR=/dlc \
    CONFIG_DIR=/config \
    STATIC_DIR=/app/static \
    PLUGINS_DIR=/app/plugins \
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
