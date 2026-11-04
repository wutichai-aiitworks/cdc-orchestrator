# PostgreSQL remote dump / dump→restore sidecar.
# pg_dump must be >= the server's major version; a newer client dumps older servers
# (back to 9.2), so track the newest PostgreSQL major here.
ARG PG_MAJOR=18
FROM postgres:${PG_MAJOR}

USER root
RUN apt-get update \
  && apt-get install -y --no-install-recommends python3 python3-paramiko curl \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY serve.py /app/serve.py

RUN mkdir -p /var/lib/pgdump/jobs \
  && chown -R postgres:postgres /var/lib/pgdump /app

USER postgres
ENV PGDUMP_WORK_ROOT=/var/lib/pgdump
ENV PGDUMP_API_HOST=0.0.0.0
ENV PGDUMP_API_PORT=8089

EXPOSE 8089
HEALTHCHECK CMD curl -fsS http://127.0.0.1:8089/health || exit 1

# Override the postgres image entrypoint: this container runs no server, only the clients.
ENTRYPOINT []
CMD ["python3", "/app/serve.py"]
