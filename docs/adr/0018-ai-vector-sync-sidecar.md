# ADR 0018 — AI vector sync sidecar (pydebeziumai)

Status: Accepted (experimental: pydebeziumai is 0.1.0)
Owner: TBD
Last updated: 2026-11-14 (amended: chunking + Docling)

## Context

AI agents and RAG search need vector collections that follow the source tables, not nightly
re-index jobs. The pipelines already put every change on Kafka (Debezium envelopes, Avro, topics
`<prefix>.<schema>.<table>`; incremental JDBC topics are flat rows). pydebeziumai (Apache-2.0,
`debezium/debezium-ai-python`) turns Debezium events into LangChain documents and writes them to
PGVector/Chroma/Milvus.

pydebeziumai 0.1.0 is a first release. Its documented `LiveContext` quickstart does not exist in the
wheel. `SyncManager` retries a failing store a few times, then dead-letters the events in memory and
returns normally. Committing Kafka offsets after it returns loses data during any store or model
outage. Its embedded engine (`pydbzengine`, a JVM inside Python) would open a second replication slot
next to Kafka Connect's.

A first attempt (2026-11-09) was rolled back for those reasons. This ADR is the second, deliberate
design.

## Decision

1. **Sidecar `cdc-vectorsync`** (`docker/vectorsync/`, Python 3.12, port 8092). It consumes the
   pipelines' **existing topics** (its own consumer group per sync). It adds no load and no
   replication slot on the source.
2. **pydebeziumai only through `bridge.py`**, using its event model, id strategies, projection
   policy, document builder and `PGVectorAdapter`. `SyncManager` and `pydbzengine` are not used.
   Versions are pinned in `requirements.txt`. A bump means re-running `test_vectorsync.py` against
   the new wheel.
3. **Delivery is ours, at-least-once and idempotent.** Document ids are stable per row
   (`schema.table:pk`), and writes are upserts. Errors are classified before commit:
   - *data* errors (undecodable record, no key, template/build failure, embedding 400/413/422 for
     one row) go to a bounded JSONL dead-letter file (location + key + error, **no row values**).
     The offset commits and the sync continues.
   - *infrastructure* errors (Kafka, schema registry, embedding server, Postgres) rewind the batch
     (seek to its first offsets) and back off up to 60 s. Nothing is committed.
   Each batch is compacted by document id (last change wins).
4. **Decoding** is Confluent-framed Avro, with the schema read from the registry REST API and cached
   by id. A value that is not Avro is tried as JSON. Debezium time types and VariableScaleDecimal
   become readable values, because this text is what gets embedded.
5. **The orchestrator owns the definitions.** They live in rqlite (kind `vectorsync_profile`).
   Secrets (target password, API key) are AES-GCM encrypted with `ORCH_SECRET_KEY` when it is set
   (ADR 0015 secretbox) and redacted on read. The sidecar keeps specs in memory only. A reconcile
   loop (`ORCH_VECTORSYNC_RECONCILE_SECS`, default 30) re-sends them after a sidecar restart and
   stops paused or deleted syncs. Every CRUD call also applies immediately.
6. **Rebuild = new generation.** The consumer group is `cdc-vectorsync-<id>-g<N>`. Rebuild, or a
   change of model, dimensions or collection, bumps N: the collection is cleared once (generation
   recorded in the sidecar volume), the new group starts at `earliest`, and old groups are deleted.
   Rows older than topic retention cannot come back. The UI flags `trimmed` topics and points to the
   pipeline's Resync.
7. **One switch.** `VECTORSYNC_ENABLED=true` in `docker/.env` does three things: `compose.inc.sh`
   adds `--profile vectorsync`, `deploy.sh` builds and starts the sidecar (and stops it again when
   the flag is false), and the orchestrator reports capability `vectorsync`, which shows the
   "AI Vector Sync" menu and `/vectorsync` page. Off means the page and API return 404, except
   `/cdc/vectorsync/capability`, which explains how to turn it on. `VECTORSYNC_DEMO=true` adds
   `cdc-pgvector` (pgvector/pgvector:0.8.0-pg17, host port 25434). Production uses the same flag
   (`docker/production/deploy.sh` adds the profile; token in `secrets/orch_vectorsync_token`).
8. **Embedding providers:** `ollama` (default `bge-m3`, multilingual), `openai` (any
   `/v1/embeddings` server) and `hash` (demo, not semantic). All are plain HTTP, so the image needs
   no provider SDKs.
9. **UX:** a five-step wizard where every step can be checked before moving on. Tables show
   retained record counts and partial history. Model and store have Test buttons that run in the
   sidecar. Documents get content modes (all / pick / template) with a live preview of real records,
   and credential-looking columns are excluded by default. The detail page shows lag, documents,
   retries with the reason, dead letters, a search playground, and a LangChain snippet.

## Consequences

- One more image (`cdc-vectorsync`, ~400 MB with langchain/psycopg/librdkafka) when enabled.
- Vectors use LangChain's PGVector tables (`langchain_pg_collection`, `langchain_pg_embedding`),
  so any LangChain/LangGraph client can read them. The schema is langchain-postgres's, not ours.
- Soft deletes need full "before" rows (Postgres `REPLICA IDENTITY FULL`).
- Specs exist in sidecar memory only, so a sidecar restart leaves syncs idle until the next
  reconcile tick (≤ 30 s). Offsets are in Kafka, so nothing is re-embedded.

## Rejected

- **pydebeziumai `SyncManager` + Kafka commits:** loses events on outages (see Context).
- **Embedded engine (`pydbzengine`):** a second slot or binlog reader per source, and a JVM in the
  sidecar.
- **Sidecar owns the definitions:** secrets would sit on the sidecar volume, and the other tools
  keep definitions in the orchestrator.
- **Resetting offsets for rebuild:** needs the group to be idle and races a running consumer. A new
  generation is stateless.

## Revisit when

- pydebeziumai ships a stable API (`LiveContext`, a commit-safe sync); then reconsider `bridge.py`.
- Another target (Chroma, Milvus, Qdrant) or a non-Kafka source is needed.

## Amendment (2026-11-14): chunking and Docling documents

**Decision.**

1. **Per table, a row can become several vectors.** `projection.tables.<t>.chunking = {enabled,
   max_chars (200-20000, default 1500), overlap (≤ half, default 150)}` splits long text on paragraph,
   line, sentence and word boundaries (`chunking.py`, no tokenizer download).
2. **Docling documents.** `projection.tables.<t>.document = {column, filename_column?,
   include_row_text?}`: the column holds the file bytes (bytea/BLOB/VARBINARY, base64, or `\x` hex). The
   format comes from the filename column or the file's magic bytes (PDF, DOCX, PPTX, XLSX, HTML,
   Markdown, CSV, PNG/JPEG/TIFF). Docling converts it (OCR off, table structure on), and
   `HierarchicalChunker` splits by sections and tables, each chunk prefixed with its heading path.
   Chunks are merged and split to `max_chars`. The file column is always excluded from content and
   metadata. A bad, oversized (`VECTORSYNC_DOCLING_MAX_MB`, default 50) or text-less file is a data
   error (dead letter). Conversions run one at a time.
3. **Identity.** Chunk ids are `<row id>#c<n>`, and every vector (chunked or not) carries
   `_parent_id` = the row id. Before a row is written, all vectors with its `_parent_id` are deleted
   (one `DELETE … cmetadata->>'_parent_id' = ANY(…)` per batch), and a hard delete removes them too. A
   shorter new version, or turning chunking off, therefore leaves nothing stale. A soft delete keeps one
   marked tombstone, not the chunks. Chunk metadata adds `_chunk`, `_chunks`, `_content`
   (`text` | `document`) and `_doc_format`, `_doc_pages`, `_doc_bytes`.
4. **Separate image, one switch.** `VECTORSYNC_DOCLING=true` builds the sidecar with `WITH_DOCLING=true`
   (dev) or runs `cdc-vectorsync-docling` (production `deploy.sh`; CI builds both). That adds Docling
   2.150 with CPU-only PyTorch and the layout and tableformer models baked into
   `/opt/docling-models`, with `HF_HUB_OFFLINE=1`, so nothing downloads at runtime. The default image
   is unchanged. The capability reports `docling.available`, and the UI offers "Document (Docling)" only then.

**Consequences.** The Docling image is about 3-4 GB and wants about 4 GB RAM. The first PDF loads the
models (~15 s), later ones take seconds per page on CPU. Kafka messages carry whole files, so files
over ~1 MB need larger `max.request.size` / `message.max.bytes` on Connect and the broker. Scanned PDFs
yield little text until OCR is enabled (a future option: the easyocr/rapidocr models are not baked in).
Docling's own tests run where it is installed (`DoclingTests`). CI's sidecar job skips them to avoid
pulling PyTorch on every run.

**Rejected.** *HybridChunker* needs a Hugging Face tokenizer download at runtime. *Docling in the
default image* would make every vector sync deployment carry PyTorch. *URL or S3 sources* were not
chosen for v1: bytes in the row need no extra credentials and follow the row's lifecycle.
