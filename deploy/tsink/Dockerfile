FROM rust:1.94-bookworm AS builder
WORKDIR /build
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev cmake clang protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --release -p tsink-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/tsink-server /usr/local/bin/tsink-server
EXPOSE 9201
ENTRYPOINT ["/usr/local/bin/tsink-server"]
