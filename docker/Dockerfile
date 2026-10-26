FROM rust:1.97-bookworm AS builder
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake zlib1g-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY build.rs ./
COPY proto ./proto
COPY src ./src

RUN cargo build --release --bins

FROM debian:bookworm-slim AS runtime
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl docker.io \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/hyper-cdc-orchestrator /usr/local/bin/hyper-cdc-orchestrator
COPY --from=builder /app/target/release/cdc-processor /usr/local/bin/cdc-processor

ENV ORCH_HOST=0.0.0.0
ENV ORCH_PORT=9788
EXPOSE 9788

ENTRYPOINT ["/usr/local/bin/hyper-cdc-orchestrator"]
