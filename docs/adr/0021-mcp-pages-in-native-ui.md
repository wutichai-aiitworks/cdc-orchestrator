# ADR 0021 — MCP pages in the native UI, and a cheap typed-route map

Status: Accepted (built 2026-11-15)
Owner: Wutichai

## Context

Operators asked for an **MCP** menu in the native UI: is mcp-cdc up, how is it configured, how do
chat clients connect, which tools exist, and what did MCP clients do. Until now that information
lived only in `mcp/README.md` and in the server's own `tools/list`, which needs an MCP client key.

While adding the pages' server route, `nuxi typecheck` failed with TS2589 ("type instantiation is
excessively deep") in unrelated files. Each added route moved the failure somewhere else, and
earlier work had already worked around it twice (catch-all proxies, `useAsyncData` in `app.vue`).

## Decision

1. **`GET /info` on mcp-cdc** returns non-secret metadata only:
   - version, the full tool catalog (name, description, read-only flag, input schema) and prompts;
   - the write switch, OAuth (enabled, public URL), rate limit, session limits;
   - client counts **by scope** and the number of active sessions.

   It never returns keys, tokens or client names. It is guarded by the **orchestrator token**,
   which the native UI server (`ORCHESTRATOR_TOKEN`) and mcp-cdc (`CDC_ORCHESTRATOR_TOKEN`) already
   share, so no new secret is needed. When that token is unset (local dev) it is open, like the
   orchestrator itself.
2. **The UI reads it through `GET /api/ui/mcp`.** `server/middleware/admin-auth.ts` guards this path
   (signed-in users only), and the server reads the mcp-cdc address from `MCP_URL` (default
   `http://mcp-cdc:8410`). mcp-cdc is optional, so "not running" is a normal answer, not an error.
3. **Pages** (sidebar group **MCP**, collapsed by default):
   - **Overview** (`/mcp`): status tiles, connection snippets per client (Claude Code, claude.ai /
     Desktop, ChatGPT, Cursor / VS Code) with `<your MCP key>` placeholders, how keys are managed,
     and the daily production check prompt.
   - **Tools** (`/mcp/tools`): searchable, grouped (`utils/mcpTools.ts`, same groups as
     `docs/mcp-tools.md`), Read/Write filter, expandable arguments, example prompts and prompts.
   - **Activity** (`/mcp/activity`): orchestrator audit events whose actor starts with `mcp:`.

   Keys stay in the server environment. The UI explains how to change them but cannot edit them,
   so no secret flows through the browser.
4. **Audit filter by actor prefix:** `GET /cdc/audit/events?actor=mcp:`. It uses
   `substr(actor, 1, length(?)) = ?`, parameterized, so `%` and `_` are not wildcards. mcp-cdc's
   `cdc_audit_events` tool takes the same `actor` argument.
5. **Cheap typed-route map** (`cdc-native-ui/nuxt.config.ts`, Nitro hook `types:extend`):
   - Nitro types every route's response as `Simplify<Serialize<Awaited<ReturnType<handler>>>>`.
     The hook replaces that with `any` and keeps the route keys, so paths and methods are still
     checked.
   - It also drops catch-all keys (`/**:path`) from the map. Template-literal matching against those
     keys was the actual depth cost: the failures began when catch-alls were added.
   - Callers already pass the response type (`$fetch<T>`), so no real typing is lost. New routes no
     longer risk TS2589.
6. **The active nav item is the most specific matching link** (`/mcp/tools` lights Tools only, not
   also Overview at `/mcp`), matched on a path boundary.

## Consequences

- `GET /info` exposes the tool catalog to anyone holding the orchestrator token. That token already
  grants the whole orchestrator API, so this widens nothing.
- Reads through MCP are not audited (only orchestrator actions are), so Activity shows changes and
  actions, not questions.
- Calls to catch-all routes are no longer path-checked by TypeScript. They still work at runtime.

## Rejected

- **A dedicated `MCP_INFO_TOKEN`:** one more secret to rotate, for metadata the orchestrator token
  already covers.
- **Calling mcp-cdc `tools/list` from the UI with an MCP client key:** the UI would hold an MCP key,
  and every page view would create an MCP session.
- **Editing keys from the UI:** keys would pass through the browser and the UI would need write
  access to the secret store. Keys stay env/secret-file managed (ADR 0008).
- **More catch-all routes / `plainFetch` per call site:** both moved the TS2589 failure around
  instead of removing its cause.
