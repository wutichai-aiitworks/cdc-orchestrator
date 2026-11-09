# ADR 0008 — mcp-cdc: per-client keys, spec-correct errors, bounded state

Status: Accepted
Owner: TBD
Last updated: 2026-11-05

## Context

mcp-cdc started as a local prompting surface for one operator: one shared `MCP_BEARER_TOKEN`, a
global `ALLOW_WRITE_TOOLS` switch, and a hand-rolled Streamable HTTP endpoint. Serving other
clients (other teams' agents, other MCP hosts) exposed gaps:

- Failed tool calls returned HTTP 400 `{"error"}` instead of a JSON-RPC result, so strict clients
  saw a transport failure and the model never read the error.
- One key for everyone: no read-only clients, no per-client revocation, and every orchestrator
  audit entry from MCP had actor `bearer`.
- A placeholder key only logged a warning; the compose fallback (30 chars) even passed the
  length check.
- Session ids were stored forever (no TTL, no cap, never checked), and rate-limit buckets were
  never pruned and were keyed by IP (one bucket for everyone behind a proxy).
- `GET /mcp` opened a keep-alive SSE stream with no authentication.
- Several pass-through tools could return secrets (Kafka Connect `database.password`,
  conninfo in logs, pgcopy create/update answers).

## Decision

1. **Per-client keys with a scope.** `MCP_CLIENT_KEYS=name:scope:token,...` (scope `read` |
   `write`; the token may contain `:`). `MCP_BEARER_TOKEN` stays as client `default` with write
   scope, so existing setups keep working. Names are `[A-Za-z0-9-_.]{1,64}`; duplicate names or
   shared tokens fail startup. Keys are compared in constant time against every entry.
2. **Writes need both** `ALLOW_WRITE_TOOLS=true` (global kill switch) **and** a write-scoped key.
   `guard_write` reports which of the two is missing. `cdc_help.write_tools_enabled` reflects the
   calling client.
3. **Audit identity.** The client name travels as `x-user: mcp:<name>` on every orchestrator call;
   the orchestrator already prefers `x-user` for its audit actor.
4. **Weak keys stop startup** (< 24 chars, or containing `change-me`/`changeme`/`placeholder`).
   `MCP_ALLOW_WEAK_TOKENS=true` is the explicit local-dev escape (set in `.env_local`).
   `docker/install.sh` generates `MCP_BEARER_TOKEN` when it is missing or a placeholder.
5. **Spec-correct errors.** Tool failures are `result: {content, isError: true}`; unknown tool and
   bad params are JSON-RPC `-32602`; malformed JSON is `-32700` with HTTP 400; auth stays HTTP 401
   (`WWW-Authenticate: Bearer`) and rate limiting HTTP 429 (`Retry-After`).
6. **Bounded sessions bound to their client.** Created on `initialize`, idle TTL
   (`MCP_SESSION_TTL_SECS`, default 24 h), cap (`MCP_MAX_SESSIONS`, default 1000, oldest evicted).
   A request carrying an unknown, expired or other client's session id gets 404 (the client
   re-initializes); requests without a session id are still accepted. `DELETE` needs a key and
   only removes the caller's own session.
7. **No server-initiated stream:** `GET /mcp` is 405, which the spec allows.
8. **Rate limit per client** (sliding minute, `MCP_RATE_LIMIT_RPM`), idle buckets pruned.
9. **Central redaction** (`mcp/src/redact.rs`) over every tool result and error text: secret-named
   keys at any depth, `password=`-style pairs and `scheme://user:pass@` in free text.
10. **No guessed credentials.** The `demo` password default applies only to the bundled demo hosts
    (`cdc-postgres-source` / `cdc-postgres-target`); other hosts must be given a password.
11. **Protocol negotiation:** answer with the client's `protocolVersion` when supported
    (2025-06-18, 2025-03-26, 2024-11-05), else the newest.
12. **Timeouts:** engine actions, profile runs and sidecar connection tests get 300 s; other calls
    keep the 60 s client timeout.
13. **Default bind** for a bare binary is `127.0.0.1:8410`; compose binds `0.0.0.0` inside the
    container and publishes on `${MCP_CDC_BIND:-127.0.0.1}`.

## Consequences

- Upgrading with a placeholder `MCP_BEARER_TOKEN` makes `mcp-cdc` exit at startup with a clear
  message; re-run `docker/install.sh` or set a real key.
- Sessions are in memory: a restart answers 404 to old session ids and clients re-initialize.
  Several replicas behind a load balancer need sticky routing on `mcp-session-id` (or clients that
  re-initialize on 404).
- TLS is still terminated by a proxy; mcp-cdc speaks plain HTTP.
- Keys are static env configuration; rotating one client means editing `MCP_CLIENT_KEYS` and
  restarting mcp-cdc.

## Rejected

- **OAuth 2.1 authorization server (MCP auth spec).** Right for public multi-tenant servers; heavy
  for an internal control plane whose clients are a handful of agents. Revisit when a client must
  authenticate as an end user rather than as a service.
- **Hashed keys in env.** Same process holds both; it would protect only against env dumps, which
  already expose `CDC_ORCHESTRATOR_TOKEN`.
- **Per-tool scopes.** Read/write covers the current clients; tool-level ACLs can extend the scope
  field later without changing the key format.
- **Keeping the 400-on-tool-error behaviour behind a flag.** No client depends on it; lenient
  clients already accept spec-shaped results.
