# syntax=docker/dockerfile:1
FROM rust:1.100-bookworm AS builder
WORKDIR /build
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY seed ./seed
COPY assets ./assets
ENV CARGO_BUILD_JOBS=1
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 tsp \
    && mkdir -p /data /app/web \
    && chown -R tsp:tsp /data /app

WORKDIR /app
COPY --from=builder /build/target/release/xingtu-tsp /usr/local/bin/xingtu-tsp
COPY web ./web

USER tsp
ENV TSP_BIND=0.0.0.0:8090 \
    TSP_JT808_ADDR=0.0.0.0:18080 \
    TSP_DATABASE=/data/tsp.db \
    TSP_WEB_DIR=/app/web \
    TSP_DEMO_MODE=true \
    RUST_LOG=xingtu_tsp=info,tower_http=warn

EXPOSE 8090 18080
VOLUME ["/data"]
CMD ["xingtu-tsp"]
