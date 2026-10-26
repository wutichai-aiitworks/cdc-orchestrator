FROM mongo:7

USER root
RUN apt-get update \
  && apt-get install -y --no-install-recommends python3 curl \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY serve.py /app/serve.py

RUN mkdir -p /var/lib/mongocopy/jobs

ENV MONGOCOPY_WORK_ROOT=/var/lib/mongocopy
ENV MONGOCOPY_API_HOST=0.0.0.0
ENV MONGOCOPY_API_PORT=8088

EXPOSE 8088
HEALTHCHECK CMD curl -fsS http://127.0.0.1:8088/health || exit 1

CMD ["python3", "/app/serve.py"]
