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
RUN cargo build --release --locked --bin polybot_convert_rust --bin copy_collect --bin clickhouse_push

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

RUN useradd --system --create-home --home-dir /home/polybot --uid 10001 polybot \
    && mkdir -p /app/data /app/output /app/logs /app/signals /app/state \
    && chown -R polybot:polybot /app

COPY --from=builder /app/target/release/polybot_convert_rust /usr/local/bin/polybot_convert_rust
COPY --from=builder /app/target/release/copy_collect /usr/local/bin/copy_collect
COPY --from=builder /app/target/release/clickhouse_push /usr/local/bin/clickhouse_push

USER polybot

ENTRYPOINT ["/usr/local/bin/polybot_convert_rust"]
