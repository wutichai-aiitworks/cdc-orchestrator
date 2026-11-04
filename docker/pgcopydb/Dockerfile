# pgcopydb on the official PostgreSQL image, so pg_dump/pg_restore match PG_MAJOR.
# PG_MAJOR must be >= every source server's major version (pg_dump refuses newer servers).
# (Was dimitri/pgcopydb:latest, which ships only a PostgreSQL 16 client.)
ARG PG_MAJOR=18
FROM postgres:${PG_MAJOR}

RUN apt-get update \
  && apt-get install -y --no-install-recommends pgcopydb python3 curl \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY serve.py /app/serve.py
COPY --from=common jobstore.py probes.py http_api.py /app/

# uid 999 = postgres here, matching the owner of existing cdc-pgcopydb-data volumes.
RUN mkdir -p /var/lib/pgcopydb/jobs && chown -R postgres:postgres /var/lib/pgcopydb /app
USER postgres
ENV PGCOPY_WORK_ROOT=/var/lib/pgcopydb \
    PGCOPY_API_HOST=0.0.0.0 \
    PGCOPY_API_PORT=8085 \
    PYTHONDONTWRITEBYTECODE=1

EXPOSE 8085
HEALTHCHECK CMD curl -fsS http://127.0.0.1:8085/health || exit 1
ENTRYPOINT []
CMD ["python3", "/app/serve.py"]
