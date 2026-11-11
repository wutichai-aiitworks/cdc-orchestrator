# cdc-wireguard: userspace WireGuard gateway (wireguard-go + gVisor netstack), ADR 0010.
# No TUN device, no NET_ADMIN: tunnels live in this process; tools reach them via TCP forwards.
# Build stage runs natively and cross-compiles (pure Go, CGO off): no QEMU for multi-arch CI.
FROM --platform=$BUILDPLATFORM golang:1.26-alpine AS build
ARG TARGETOS TARGETARCH
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY *.go ./
RUN CGO_ENABLED=0 GOOS=$TARGETOS GOARCH=$TARGETARCH go build -trimpath -ldflags="-s -w" -o /out/cdc-wireguard .

FROM alpine:3.21
RUN apk add --no-cache ca-certificates && adduser -D -H -u 10091 wg
COPY --from=build /out/cdc-wireguard /usr/local/bin/cdc-wireguard
USER wg
ENV WG_API_PORT=8091 WG_FORWARD_PORT_MIN=21000 WG_FORWARD_PORT_MAX=21999 WG_FORWARD_HOST=cdc-wireguard
EXPOSE 8091
HEALTHCHECK --interval=15s --timeout=3s CMD wget -qO- http://127.0.0.1:8091/health >/dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/cdc-wireguard"]
