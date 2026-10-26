FROM dimitri/pgcopydb:latest

USER root
RUN apt-get update \
  && apt-get install -y --no-install-recommends python3 curl \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY serve.py /app/serve.py

RUN mkdir -p /var/lib/pgcopydb/jobs \
  && chown -R docker:postgres /var/lib/pgcopydb /app

USER docker
ENV PGCOPY_WORK_ROOT=/var/lib/pgcopydb
ENV PGCOPY_API_HOST=0.0.0.0
ENV PGCOPY_API_PORT=8085

EXPOSE 8085
HEALTHCHECK CMD curl -fsS http://127.0.0.1:8085/health || exit 1

CMD ["python3", "/app/serve.py"]
