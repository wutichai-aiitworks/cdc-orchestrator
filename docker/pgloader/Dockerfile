FROM python:3.12-slim-bookworm

RUN apt-get update \
  && apt-get install -y --no-install-recommends pgloader curl \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY serve.py /app/serve.py

RUN mkdir -p /var/lib/pgloader/jobs

ENV PGLOADER_WORK_ROOT=/var/lib/pgloader
ENV PGLOADER_API_HOST=0.0.0.0
ENV PGLOADER_API_PORT=8087

EXPOSE 8087
HEALTHCHECK CMD curl -fsS http://127.0.0.1:8087/health || exit 1

CMD ["python3", "/app/serve.py"]
