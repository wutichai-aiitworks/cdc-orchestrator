# Upstream publishes only `latest`; pin the digest so release builds are reproducible.
# Bump: docker buildx imagetools inspect ghcr.io/tair-opensource/redisshake:latest
FROM ghcr.io/tair-opensource/redisshake:latest@sha256:ce1770f7af3b4dba6d87625c7d1d1078fd6c9dc0cbd136c1e87f87a8d8b31382

USER root
RUN apk add --no-cache python3 curl

WORKDIR /app
COPY serve.py /app/serve.py
COPY --from=common jobstore.py probes.py http_api.py /app/

RUN mkdir -p /var/lib/redisshake/jobs \
  && chmod +x /app/serve.py

ENV REDIS_SHAKE_WORK_ROOT=/var/lib/redisshake
ENV REDIS_SHAKE_API_HOST=0.0.0.0
ENV REDIS_SHAKE_API_PORT=8086
ENV REDIS_SHAKE_BIN=/app/redis-shake

EXPOSE 8086
HEALTHCHECK CMD curl -fsS http://127.0.0.1:8086/health || exit 1

ENTRYPOINT ["python3", "/app/serve.py"]
