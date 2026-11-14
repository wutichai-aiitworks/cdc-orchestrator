# cdc-vectorsync: CDC topics → embeddings → PGVector (ADR 0018). pydebeziumai needs Python <3.13.
FROM python:3.12-slim-bookworm

ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1 PIP_NO_CACHE_DIR=1 PIP_DISABLE_PIP_VERSION_CHECK=1
RUN apt-get update \
  && apt-get install -y --no-install-recommends curl \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY requirements.txt requirements-docling.txt /app/
RUN pip install -r /app/requirements.txt

# Optional Docling (VECTORSYNC_DOCLING=true → WITH_DOCLING=true): PDF/DOCX/PPTX/XLSX/HTML in a
# bytes column → structure-aware chunks. CPU-only PyTorch (the default Linux wheels add ~2 GB of
# CUDA). Layout + table models are baked in and Hugging Face runs offline: no downloads at runtime.
ARG WITH_DOCLING=false
ENV DOCLING_ARTIFACTS_PATH=/opt/docling-models
RUN if [ "$WITH_DOCLING" = "true" ]; then \
      pip install --extra-index-url https://download.pytorch.org/whl/cpu -r /app/requirements-docling.txt \
      && docling-tools models download layout tableformer -o /opt/docling-models \
      && chmod -R a+rX /opt/docling-models; \
    fi
ENV HF_HUB_OFFLINE=1 TRANSFORMERS_OFFLINE=1 OMP_NUM_THREADS=4

COPY bridge.py chunking.py decode.py docling_tx.py embeddings.py engine.py serve.py /app/
RUN useradd --system --uid 10092 --home /var/lib/vectorsync vectorsync \
  && mkdir -p /var/lib/vectorsync/dead-letters \
  && chown -R vectorsync /var/lib/vectorsync

USER vectorsync
ENV VECTORSYNC_WORK_ROOT=/var/lib/vectorsync VECTORSYNC_API_HOST=0.0.0.0 VECTORSYNC_API_PORT=8092
EXPOSE 8092
HEALTHCHECK CMD curl -fsS http://127.0.0.1:8092/health || exit 1
ENTRYPOINT ["python3", "/app/serve.py"]
