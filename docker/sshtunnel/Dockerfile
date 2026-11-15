# cdc-sshtunnel: SSH tunnel gateway (golang.org/x/crypto/ssh) with optional Cloudflare Access, ADR 0020.
# Tools reach databases behind SSH-only sites as cdc-sshtunnel:<listen_port> (stable ports 22000-22999).
# Build stage runs natively and cross-compiles (pure Go, CGO off): no QEMU for multi-arch CI.
ARG CLOUDFLARED_VERSION=2025.4.0

FROM --platform=$BUILDPLATFORM golang:1.26-alpine AS build
ARG TARGETOS TARGETARCH
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY main.go ./
RUN CGO_ENABLED=0 GOOS=$TARGETOS GOARCH=$TARGETARCH go build -trimpath -ldflags="-s -w" -o /out/cdc-sshtunnel .

# Official multi-arch image; only the static binary is copied.
FROM cloudflare/cloudflared:${CLOUDFLARED_VERSION} AS cloudflared

FROM alpine:3.21
RUN apk add --no-cache ca-certificates && adduser -D -H -u 10092 ssht
COPY --from=build /out/cdc-sshtunnel /usr/local/bin/cdc-sshtunnel
COPY --from=cloudflared /usr/local/bin/cloudflared /usr/local/bin/cloudflared
USER ssht
ENV SSHT_API_PORT=8092 SSHT_FORWARD_PORT_MIN=22000 SSHT_FORWARD_PORT_MAX=22999 SSHT_FORWARD_HOST=cdc-sshtunnel
EXPOSE 8092
HEALTHCHECK --interval=15s --timeout=3s CMD wget -qO- http://127.0.0.1:8092/health >/dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/cdc-sshtunnel"]
