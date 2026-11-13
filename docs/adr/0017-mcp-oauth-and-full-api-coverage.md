# ADR 0017 — mcp-cdc: OAuth for remote connectors, and every /cdc operation reachable

Status: Accepted (built; verified end to end against a local stack)
Owner: Wutichai
Last updated: 2026-11-12
Amends: ADR 0008 ("Rejected: OAuth 2.1 … revisit when a client must authenticate as an end user")

## Context

Operators want to run the CDC stack by prompting from Claude (Code, Desktop, claude.ai),
ChatGPT and other MCP clients.

- **Header clients** — Claude Code, Cursor, VS Code, mcp-remote — can send
  `Authorization: Bearer <key>`, so ADR 0008's per-client keys already work for them.
- **Remote connectors in claude.ai and ChatGPT** cannot set a header. They speak the MCP
  authorization spec (OAuth 2.1 with discovery, dynamic client registration and PKCE) or no
  auth at all. That is the "revisit" trigger in ADR 0008.

Separately, the MCP server only covered about 65 of the orchestrator's 142 routes. It had no
tools for ADR 0015 (channels, deliveries) or ADR 0016 (latency), and every pipeline tool
needed a UUID, which people do not type in a prompt.

## Decision

1. **A thin OAuth bridge, not a user system.**
   - **Identity stays the ADR 0008 client key.** On the consent page an operator pastes one of
     `MCP_CLIENT_KEYS`. The app then acts as that client (audit `actor=mcp:<name>`) with that
     scope, or with read only if the app asked only for `cdc:read`.
   - **Revocation is immediate.** Each request re-checks that the key still exists and still
     has the scope, so removing or downgrading a key revokes every grant made from it.
   - **Endpoints:**
     - `/.well-known/oauth-protected-resource[/mcp]` (RFC 9728);
     - `/.well-known/oauth-authorization-server[/mcp]` and `/.well-known/openid-configuration`
       (RFC 8414);
     - `/oauth/register` (RFC 7591);
     - `/oauth/authorize` (consent page);
     - `/oauth/token` (authorization_code + refresh_token).
     A 401 on `/mcp` carries `WWW-Authenticate: Bearer resource_metadata=…` so clients find
     them.
   - **Rules:**
     - PKCE S256 only;
     - redirect URIs must be https, or http on loopback;
     - codes are single use and expire in 5 minutes;
     - access tokens last 1 h; refresh tokens last 30 days and rotate on use;
     - wrong keys lock the client IP out for 15 minutes after 10 attempts;
     - the consent page cannot be framed.
   - **Storage.** Tokens are random 256-bit values stored only as SHA-256 hashes in
     `MCP_OAUTH_STORE` (compose volume `cdc-mcp-data`), so grants survive a restart.
   - **Public URL.** `MCP_PUBLIC_URL` is the https URL clients use. Without it the URL is derived
     from `X-Forwarded-*` / `Host`. `MCP_OAUTH=false` turns the bridge off.
2. **Every documented operation is reachable.**
   - `scripts/generate-openapi.sh` writes `mcp/api-catalog.json` (method, path, summary, tag,
     query parameters, read/write) and the binary embeds it.
   - `cdc_api_catalog` searches the catalog.
   - `cdc_api_get` calls any `/cdc/...` GET.
   - `cdc_api_call` calls any POST, PUT, PATCH or DELETE. It sits behind the same write gate as
     every other write tool (`ALLOW_WRITE_TOOLS` and a write-scoped key).
   - Paths must start with `/cdc/` and contain no `..`, `//`, `?` or unfilled `{param}`.
3. **Dedicated tools for the common questions.**
   - Latency: `cdc_latency`, `cdc_pipeline_latency` (summary with p50/p95/max and at most 60
     points), `cdc_latency_probe` (can restart the pipeline).
   - Channels and deliveries: `cdc_notification_channels_list`,
     `cdc_notification_channel_upsert`, `…_test`, `…_delete`, `cdc_notification_deliveries`.
   - History: `cdc_audit_events`, `cdc_pipeline_logs`.
4. **Pipelines by name.** Every tool that takes a pipeline accepts its UUID or its name: an exact
   case-insensitive match, else a unique partial match. Ambiguous names list the candidates.

## Consequences

- claude.ai and ChatGPT need the server on a public https URL (TLS proxy or Cloudflare
  tunnel). The consent page plus the key are the only gate: anyone who can reach the URL can
  register a client, but cannot get a token without a key.
- Channel secrets typed into a chat pass through the model provider. Tool output is still
  redacted (ADR 0008), and the orchestrator never returns secrets (ADR 0015). Prefer the WebUI
  for tokens and passwords.
- `cdc_api_call` makes every write reachable. Destructive ones (delete, stop, resync, clear)
  rely on the client asking the user; the server instructions say to confirm first. Keep
  `ALLOW_WRITE_TOOLS=false` or read-scoped keys where that is not acceptable.

## Not in this change

- Per-tool scopes or a deny-list for `cdc_api_call`.
- Server-sent events on GET `/mcp` (responses stay JSON; clients that need SSE fall back).
- OAuth user accounts / SSO login on the consent page.
