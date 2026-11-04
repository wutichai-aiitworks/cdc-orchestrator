FROM python:3.12-slim-bookworm

RUN apt-get update \
  && apt-get install -y --no-install-recommends pgloader postgresql-client curl \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY serve.py /app/serve.py
# psql: Test connection for the PostgreSQL target. Shared job store / HTTP contract / probes:
COPY --from=common jobstore.py probes.py http_api.py /app/

RUN mkdir -p /var/lib/pgloader/jobs

ENV PGLOADER_WORK_ROOT=/var/lib/pgloader
ENV PGLOADER_API_HOST=0.0.0.0
ENV PGLOADER_API_PORT=8087
ENV PYTHONDONTWRITEBYTECODE=1

EXPOSE 8087
HEALTHCHECK CMD curl -fsS http://127.0.0.1:8087/health || exit 1

CMD ["python3", "/app/serve.py"]
