# syntax=docker/dockerfile:1.7

FROM rust:1-slim-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY vendor ./vendor
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

RUN useradd --system --create-home --home-dir /home/polybot --uid 10001 polybot \
    && mkdir -p /app/data /app/output /app/logs /app/signals /app/state \
    && chown -R polybot:polybot /app

COPY --from=builder /app/target/release/polybot_convert_rust /usr/local/bin/polybot_convert_rust

USER polybot

ENTRYPOINT ["/usr/local/bin/polybot_convert_rust"]
