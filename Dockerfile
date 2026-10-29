# syntax=docker/dockerfile:1.7
#
# Ranch server: the Ranch UI on the real Rust core, without the desktop app.
#
#   docker build -t ranch .
#   docker run -p 8080:8080 -v ranch-data:/data -e RANCH_PASSWORD=… ranch
#
# Three stages: the frontend (pnpm), the server binary (cargo, built WITHOUT
# the default `desktop` feature: no Tauri, no WebKit, no libdbus), and a
# distroless runtime: no shell, no package manager, non-root.
# MCP is not part of the image: see README "Ranch server".
#
# Every base image is pinned by digest (a multi-arch index: amd64 and arm64
# both resolve), so a rebuild of the same commit uses the same bases. The Rust
# version is rust-toolchain.toml's. To bump one: change the tag, then take the
# new digest from `docker buildx imagetools inspect <image>:<tag>`.

# ── 1. frontend ───────────────────────────────────────────────────────────
FROM --platform=$BUILDPLATFORM node:24-bookworm-slim@sha256:be8204d4bd327363357f532187b752ff62759c31bedf98e9e18fc08c9cbd605c AS web
WORKDIR /src
RUN corepack enable
COPY package.json pnpm-lock.yaml ./
RUN --mount=type=cache,id=pnpm,target=/root/.local/share/pnpm/store \
    pnpm install --frozen-lockfile
COPY index.html tsconfig.json tsconfig.node.json vite.config.ts components.json ./
COPY public ./public
COPY src ./src
# The bundle is the one the desktop app ships: outside Tauri it speaks HTTP.
# `vite build`, not `pnpm build` (= `tsc && vite build`): types (test files
# included) are CI's job, not the image's.
RUN pnpm exec vite build

# ── 2. server binary ──────────────────────────────────────────────────────
FROM rust:1.99.1-bookworm@sha256:9181b56c2a21ab1d5e73cffe04fc762426d30aa7fac3c91141dd033f49a7fc8b AS server
WORKDIR /src/src-tauri
COPY src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/build.rs ./
# The workspace's other crates: the domain, the application, the secret
# stores, the SQLite stores (with the migrations), BullMQ on Redis.
COPY src-tauri/crates ./crates
COPY src-tauri/src ./src
RUN --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-target,target=/src/src-tauri/target \
    cargo build --release --locked --no-default-features --features server --bin ranch-server \
    && cp target/release/ranch-server /ranch-server \
    # The data dir is created here so that it exists, owned by the runtime
    # user, in an image that has no `mkdir`; a named volume inherits it.
    && mkdir /data

# ── 3. runtime ────────────────────────────────────────────────────────────
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f
LABEL org.opencontainers.image.title="Ranch" \
      org.opencontainers.image.description="BullMQ client: the Ranch UI served over HTTP" \
      org.opencontainers.image.licenses="MIT"

COPY --from=server /ranch-server /usr/local/bin/ranch-server
COPY --from=web /src/dist /app/dist
COPY --from=server --chown=nonroot:nonroot /data /data

ENV RANCH_DATA_DIR=/data \
    RANCH_STATIC_DIR=/app/dist \
    RANCH_BIND=0.0.0.0 \
    RANCH_PORT=8080
VOLUME ["/data"]
EXPOSE 8080
USER nonroot

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/ranch-server", "--healthcheck"]

ENTRYPOINT ["/usr/local/bin/ranch-server"]
