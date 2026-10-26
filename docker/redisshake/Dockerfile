FROM ghcr.io/tair-opensource/redisshake:latest

USER root
RUN apk add --no-cache python3 curl

WORKDIR /app
COPY serve.py /app/serve.py

RUN mkdir -p /var/lib/redisshake/jobs \
  && chmod +x /app/serve.py

ENV REDIS_SHAKE_WORK_ROOT=/var/lib/redisshake
ENV REDIS_SHAKE_API_HOST=0.0.0.0
ENV REDIS_SHAKE_API_PORT=8086
ENV REDIS_SHAKE_BIN=/app/redis-shake

EXPOSE 8086
HEALTHCHECK CMD curl -fsS http://127.0.0.1:8086/health || exit 1

ENTRYPOINT ["python3", "/app/serve.py"]
