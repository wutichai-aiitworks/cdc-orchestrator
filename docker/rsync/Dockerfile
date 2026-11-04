# Host-to-host file sync sidecar: rsync over SSH (direct via the source, or relay through here).
# Needs OpenSSH >= 8.2 for `ForwardAgent=<socket>` (bookworm ships 9.2).
FROM debian:bookworm-slim

RUN apt-get update \
  && apt-get install -y --no-install-recommends python3 rsync openssh-client sshpass curl ca-certificates \
  && rm -rf /var/lib/apt/lists/* \
  && useradd --system --uid 10090 --create-home --home-dir /home/rsync rsync

WORKDIR /app
COPY serve.py /app/serve.py

RUN mkdir -p /var/lib/rsync/jobs /var/lib/rsync/staging \
  && chown -R rsync:rsync /var/lib/rsync /app

USER rsync
ENV RSYNC_WORK_ROOT=/var/lib/rsync
ENV RSYNC_API_HOST=0.0.0.0
ENV RSYNC_API_PORT=8090

EXPOSE 8090
HEALTHCHECK CMD curl -fsS http://127.0.0.1:8090/health || exit 1

CMD ["python3", "/app/serve.py"]
