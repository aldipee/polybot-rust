# syntax=docker/dockerfile:1.7

FROM rust:1-slim-bookworm AS builder
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY build.rs ./
COPY vendor ./vendor
COPY src ./src
ARG BUILD_BINS="polybot copy_collect clickhouse_push"
RUN --mount=type=cache,id=polybot-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=polybot-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=polybot-target,target=/app/target \
    set -eux; \
    bins=""; \
    for bin in ${BUILD_BINS}; do bins="$bins --bin ${bin}"; done; \
    cargo build --release --locked ${bins}; \
    mkdir -p /artifacts; \
    for bin in ${BUILD_BINS}; do cp "/app/target/release/${bin}" "/artifacts/${bin}"; done

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata libssl3 gosu \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

RUN useradd --system --create-home --home-dir /home/polybot --uid 10001 polybot \
    && mkdir -p /app/data /app/output /app/logs /app/signals /app/state \
    && chown -R polybot:polybot /app

COPY --from=builder /artifacts/ /usr/local/bin/
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
