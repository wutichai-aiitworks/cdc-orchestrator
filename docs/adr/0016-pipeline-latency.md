# ADR 0016 — Per-pipeline latency (estimate for all, exact with a heartbeat)

Status: Accepted (built; see "Verified" / "Not verified")
Owner: Wutichai
Last updated: 2026-11-12

## Context

Operators asked "how far behind is each pipeline?". Until now the orchestrator could only
show a backlog in *messages*: sink consumer lag via `docker exec rpk`, and rows/s computed in
the browser. There was no time-based latency, no history, no alert on it, and nothing in
`/metrics`. GlueSync (see `docs/roadmap/gluesync-gap-roadmap.md`) shows per-pipeline latency,
so it is table stakes.

Writing to customer databases is not acceptable by default, but an operator should be able to
opt in to exact numbers.

## Decision

Tiered measurement. Each value carries `method` so the UI can say how exact it is.

1. **Estimate (every running pipeline, no writes).** A sampler runs every
   `ORCH_LATENCY_SAMPLE_SECS` (default 15). It reads Connect REST (`/connectors/{sink}/offsets`,
   `/connectors/{source}/offsets`, `/connectors/{source}/topics`) and Kafka directly through
   rdkafka (`ORCH_KAFKA_BOOTSTRAP`, default `cdc-message:9092`). The rdkafka consumer has its own
   group, never commits, and only reads watermarks and single records.
   - **Sink delay** is the Kafka timestamp age of the oldest record the sink has not committed.
     Connect commits sink offsets only every `offset.flush.interval.ms`
     (`ORCH_CONNECT_OFFSET_FLUSH_MS`, default 60 000), so a backlog younger than that counts as
     0 and an older one is an upper bound.
   - **Capture delay** depends on the capture mode:
     - log-based: newest record's Kafka time minus the source commit time in the Debezium
       offset (`ts_usec` / `ts_sec` / `ts_ms`);
     - polling: the poll interval.
   - **End-to-end** is capture + sink when the sink is behind, otherwise capture when new
     records arrived. When nothing arrived and nothing is waiting, it is "idle" (no number).
   - **Throughput** is the delta of the high-watermark sum per second.
2. **Trigger inbox (exact, free).** Trigger capture into a Postgres target already records
   `changed_at` and `applied_at` in `hyper_cdc.inbox`. The p50 of `applied_at − changed_at`
   over the last two sample intervals is used. It uses the two databases' clocks.
3. **Heartbeat probe (exact, opt-in per pipeline):** `latency_probe {enabled, interval_secs}`.
   - The orchestrator upserts `(pipeline_id, seq)` into `hyper_cdc.hyper_cdc_heartbeat` on the
     source.
   - It then polls the target table `<target schema>.hyper_cdc_heartbeat` every 250 ms until
     `seq` arrives.
   - Latency is first-seen minus written, both on the orchestrator's clock, so database clock
     skew does not matter. Resolution is 250 ms.
   - A heartbeat still outstanding counts as its current age, so a stall shows immediately.
   - Turning it on creates the table and adds it to the publication when the publication lists
     tables. `reconcile` then adds the table to the Debezium `schema/table.include.list` and the
     sink `topics.regex`.
   - A running pipeline needs a restart (the API returns `restartRequired`).
   - It also keeps an idle Postgres replication slot advancing.
   - Scope: Postgres source with log-based capture → Postgres target.
4. **Storage.** Raw samples are kept 6 h in memory. 5-minute rollups (p50, p95, max e2e, max
   capture/sink/lag, avg throughput, best method) go to rqlite `cdc_pipeline_metrics` and are
   kept 30 days.
5. **Surfaces.**
   - API: `GET /cdc/latency`, `GET /cdc/profiles/{id}/latency?range=`,
     `PUT /cdc/profiles/{id}/latency-probe`.
   - `/metrics` gauges: `cdc_pipeline_latency_seconds{kind=e2e|capture|sink}`,
     `cdc_pipeline_lag_records` and `cdc_pipeline_throughput_rps`.
   - UI: a chip on cards, a sortable list column, a Latency tab (sparkline, split, probe toggle)
     and an Overview "Worst latency" KPI.
6. **Alert.** Rule condition `latency_above` with `{seconds, forMinutes}`, matched against the
   pipeline name. It fires once per breach and sends a `latency_recovered` message when the
   value drops back. Delivery goes through ADR 0015 channels.

## Consequences

- Estimates are honest but coarse on the sink side (commit granularity). The UI labels them
  "Estimate" and explains why; the probe is one click away where it is supported.
- A pipeline with the probe on gets one extra table on source and target (one row per
  pipeline). Turning the probe off deletes the row but keeps the table (it may be shared).
- In-memory series, probe and breach state are per instance and reset on restart. Rollups
  survive in rqlite.
- Sampling cost per pipeline is two or three Connect calls, one metadata and watermark fetch
  per topic, and at most ten single-record reads.

## Not verified / not in this change

- Probe for MySQL / SQL Server / Oracle sources or non-Postgres targets.
- Debezium JMX metrics (`MilliSecondsBehindSource`). They need the JMX exporter (roadmap R2).
- Lake / stream patterns: only capture delay and throughput (no managed sink).
