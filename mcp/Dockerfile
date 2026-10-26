FROM rust:1.97-bookworm AS builder
WORKDIR /app
COPY Cargo.toml ./
COPY Cargo.lock* ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/mcp-cdc /usr/local/bin/mcp-cdc
ENV MCP_BIND_ADDR=0.0.0.0:8410
EXPOSE 8410
ENTRYPOINT ["/usr/local/bin/mcp-cdc"]
