# Hyper CDC — documentation

Start with the guide that matches what you are doing. Decisions and their reasons live in
[adr/](adr/); re-read the ADR before reopening a settled design question.

## Operator guides

| Guide | Use it when |
|---|---|
| [production-rollout-guideline.md](production-rollout-guideline.md) | Rolling out to a customer production source: readiness verdict and go/no-go list, impact on the source per phase (Postgres CDC, Redis sync), what to run at night, runbook, monitoring and abort criteria, rollback |
| [postgres-large-source-cdc.md](postgres-large-source-cdc.md) | Replicating a large PostgreSQL source: the slot-first rule, options A/B/C, the readiness check (UI / API / SQL), Option C step by step, a large dump over SSH |
| [ssh-tunnel-sources.md](ssh-tunnel-sources.md) | The source or target is reachable only over SSH or Cloudflare Zero Trust: site preparation, Cloudflare Access vs Direct SSH, host key pinning, pipeline setup, troubleshooting, `ORCH_SSHTUNNEL_*` configuration, API samples |
| [mcp-tools.md](mcp-tools.md) | Operating and monitoring from a chat client (Claude, ChatGPT, Cursor) through `mcp-cdc`: the web UI MCP pages, all 80 tools with read/write flags, example prompts, a daily rollout check |
| [wireguard-sites.md](wireguard-sites.md) | The site can open one UDP port for a WireGuard tunnel (Copy & Sync tools) |
| [oracle-incremental-sync-and-audit.md](oracle-incremental-sync-and-audit.md) | Oracle source without log-based capture (incremental polling) |

## Decisions (ADRs) behind these guides

| ADR | Topic |
|---|---|
| [0004](adr/0004-incremental-rolling-window-and-initial-load.md) | Incremental polling strategies and the streaming initial load |
| [0005](adr/0005-postgres-remote-dump-sidecar.md) | pg_dump / pg_restore sidecar |
| [0006](adr/0006-copy-sync-sidecars.md) | Copy & Sync sidecars (pgcopydb, RedisShake, …) |
| [0010](adr/0010-wireguard-site-tunnels.md) | WireGuard tunnels |
| [0013](adr/0013-record-diff.md) | Verify Record (source vs target by key) |
| [0014](adr/0014-cdc-start-position.md) | Start position: `earliest_retained` / custom LSN, Reposition |
| [0019](adr/0019-live-source-cdc-readiness.md) | Live readiness check ("Check source") |
| [0020](adr/0020-ssh-tunnel-connection-method.md) | Connection method: Direct or SSH tunnel |
| [0021](adr/0021-mcp-pages-in-native-ui.md) | MCP pages in the web UI, mcp-cdc `/info`, cheap typed-route map |
| [0022](adr/0022-redis-sync-supervisor.md) | Redis sync supervisor: `redis_sync_down` alerts, auto-restart policy (night window), backoff and cap |

## Other material

- [architecture/](architecture/), [design/](design/): system and UX design notes
- [production-cdc-*.md](production-cdc-hardening-plan.md): the production hardening plan and phase results
- [openapi.yaml](openapi.yaml) / [openapi.json](openapi.json): orchestrator API contract
- [testing/](testing/): test plans and reports
