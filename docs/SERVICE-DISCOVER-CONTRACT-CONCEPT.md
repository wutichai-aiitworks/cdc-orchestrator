# Gateway discover & contract — concept (draft)

**Status:** draft — product-neutral pattern for any control-plane gateway and optional workers  
**Audience:** teams adding pluggable tools, sidecars, or integration surfaces behind one HTTP API  
**Versions:** `discover_version` and `contract_version` (independent; both start at `"1"`)

---

## 1. Problem

Integrators (Web UI, agents, scripts, other services) should not hardcode:

- which optional capabilities exist in a deployment,
- worker hostnames or ports,
- configuration field names, types, and defaults,
- every HTTP path for profiles, jobs, or runs.

They should **point at one gateway** (base URL + auth), then:

1. **Discover** — what is available and healthy *here*?
2. **Contract** — what JSON and routes do I use *always*?

**Integrator configuration is reduced to:** `GATEWAY_BASE_URL` + `GATEWAY_AUTH` (Bearer, API key, or your gateway’s scheme).

This pattern is **not** a full OpenAPI replacement. It is a **thin, tool-centric** layer for optional plugins and profile/job workflows.

---

## 2. Two concepts

| | **Discovery** | **Contract** |
|---|----------------|--------------|
| **Question** | What exists? Is it enabled? What version/limits now? | How do I configure and call it? |
| **Stability** | Volatile (per deploy, per runtime) | Stable (same in dev and prod) |
| **Owner** | Gateway (+ optional worker runtime slice) | **Gateway only** (source of truth) |
| **Version field** | `discover_version` | `contract_version` |

```text
                    ┌─────────────────────────────────────┐
                    │  Client (UI, agent, script)         │
                    └─────────────────┬───────────────────┘
                                      │
          ┌───────────────────────────┼───────────────────────────┐
          │                           │                           │
          ▼                           ▼                           │
   GET …/tools/discover        GET …/{tool}/discover               │
   (catalog)                   (tool + optional worker embed)      │
          │                           │                           │
          │         contract_path     │                           │
          └──────────────────────────►│                           │
                                      ▼                           │
                    GET …/{tool}/contract  ◄── canonical schema    │
                                      │                           │
                                      ▼                           │
                    POST …/profiles, …/run, …/jobs  (from contract│
                    .operations)                                  │
                                      │                           │
                                      ▼ (optional)                │
                    Worker  GET /v1/discover  (runtime only)      │
                    Worker  POST /v1/jobs …   (execution)         │
                    └─────────────────────────────────────────────┘
                                      Gateway
```

---

## 3. When to use

| Use discover + contract when… | Skip when… |
|-------------------------------|------------|
| Capability is optional per deployment | Single fixed API, no plugins |
| Payload has many flags, enums, or connections | One small `POST` with few fields |
| Multiple clients must stay aligned | One internal caller only |
| Clients talk to a **gateway**, not workers directly | Clients call workers with no facade |

---

## Part A — Discovery

### A.1 What discovery is

Discovery answers **runtime and deployment** questions:

- Which tools are registered for this product?
- Is tool `{tool_id}` **enabled** (gateway wired to a worker)?
- Is the worker up, and what **version / limits** does it report?
- Where is the **contract** URL? (pointer only — not the schema)

Discovery must **not** include secrets, passwords, tokens, or live credentials.

### A.2 What discovery is not

| Discovery | Use **contract** instead |
|-----------|----------------------------|
| `enabled` | Field names, types, required flags |
| Worker version string | `options.defaults` |
| Runtime `limits` | `minimal_config_example` |
| Path to contract | Full `operations` list |
| Operator env **names** (not values) | Detailed validation rules |

### A.3 Three levels

#### Level 1 — Catalog discover (gateway)

**Question:** “What can I use in this environment?”  
**Owner:** Gateway only.  
**Suggested path:** `GET /{api_prefix}/tools/discover`  
(Your product may use a segment name, e.g. `…/integrations/discover`, `…/capabilities/discover`.)

**Top-level response:**

| Field | Type | Meaning |
|-------|------|---------|
| `discover_version` | string | Format of this payload (e.g. `"1"`) |
| `timestamp` | ISO8601 | When the catalog was generated |
| `tools` | array | One entry per pluggable tool |

**Each `tools[]` entry:**

| Field | Type | Meaning |
|-------|------|---------|
| `id` | string | Stable `tool_id` (same in URLs, discovery, contract) |
| `title` | string | Display name |
| `enabled` | boolean | Gateway configured to use worker (policy-defined) |
| `discover_path` | string | Gateway path for tool-level discover |
| `contract_path` | string | Gateway path for contract (required for clients) |
| `ui_path` | string | Optional front-end route |
| `gateway_env` | string[] | Env var **names** for operators (no values) |

**Example:**

```json
{
  "discover_version": "1",
  "timestamp": "2026-10-25T02:00:00Z",
  "tools": [
    {
      "id": "bulk_copy",
      "title": "Bulk copy",
      "enabled": true,
      "discover_path": "/api/v1/bulk_copy/discover",
      "contract_path": "/api/v1/bulk_copy/contract",
      "ui_path": "/tools/bulk-copy",
      "gateway_env": ["GATEWAY_BULK_COPY_URL", "GATEWAY_BULK_COPY_TOKEN"]
    }
  ]
}
```

#### Level 2 — Tool discover (gateway)

**Question:** “Is this tool usable now, and what does the worker report?”  
**Owner:** Gateway.  
**Suggested path:** `GET /{api_prefix}/{tool_id}/discover`

| Field | Type | Meaning |
|-------|------|---------|
| `discover_version` | string | `"1"` |
| `id` | string | Same as catalog |
| `enabled` | boolean | Same semantics as catalog |
| `contract_path` | string | Next step for configuration |
| `worker` | object \| null | Embedded worker discover, or `null` / error if unavailable |
| `minimal_config_example` | object | Optional copy from contract for agents; **contract remains canonical** |

If `GATEWAY_*_URL` (or equivalent) is set, gateway calls worker `GET /v1/discover` and nests under `worker`. Document behavior when worker is down (`worker: { "error": "…" }` vs `enabled: false`).

#### Level 3 — Worker discover (optional worker HTTP API)

**Question:** “Am I ready, and what runtime limits apply?”  
**Suggested path:** `GET /v1/discover` on the worker (not on the gateway).

| Field | Type | Meaning |
|-------|------|---------|
| `discover_version` | string | `"1"` |
| `tool_id` | string | Matches gateway `id` |
| `service` | string | Worker service name |
| `worker_ready` | boolean | Accepting work |
| `gateway_contract` | string | Path on **gateway** for contract (e.g. `/api/v1/my_tool/contract`) |
| `worker_version` | string | Optional binary/package version |
| `limits` | object | Optional concurrency, retention, quotas |

**Example:**

```json
{
  "discover_version": "1",
  "tool_id": "bulk_copy",
  "service": "bulk-copy-worker",
  "worker_ready": true,
  "gateway_contract": "/api/v1/bulk_copy/contract",
  "worker_version": "2.1.0",
  "limits": { "max_concurrent_jobs": 1 }
}
```

Do **not** put full option lists or profile schemas on the worker. Legacy `GET /v1/capability` may remain; new clients use **discover** + **contract**.

### A.4 Discovery naming

| Level | Pattern |
|-------|---------|
| Catalog | `GET /{api_prefix}/tools/discover` (or product-specific catalog path) |
| Tool | `GET /{api_prefix}/{tool_id}/discover` |
| Worker | `GET /v1/discover` |

Use one stable `tool_id` everywhere (avoid `my-tool` vs `my_tool` mismatch). Auth: same as other gateway read APIs unless you expose a public catalog.

---

## Part B — Contract

### B.1 What contract is

**Contract** is a **stable, versioned** document describing how to configure and invoke a tool **through the gateway**:

- Connection / endpoint field shapes
- Profile (or config) top-level fields
- Options: booleans, integers, strings, enums, **`defaults`**, optional `hints`
- **`minimal_config_example`** — valid minimal body for create
- **`operations`** — method, path, purpose for each gateway route the client needs

Contract is **deployment-independent**. Discovery tells you if the tool is `enabled` in this environment.

### B.2 What contract is not

| Contract | Use **discovery** instead |
|----------|---------------------------|
| Schema and defaults | `enabled`, worker version |
| `operations` | Catalog of all tools |
| `contract_version` | `discover_version` |

No secrets, live credentials, or worker base URLs in contract (gateway-relative paths only).

**Gateway owns contract.** Workers should not host the canonical copy unless the gateway is a pure proxy with no schema knowledge.

### B.3 Suggested path

`GET /{api_prefix}/{tool_id}/contract`

### B.4 Contract sections

| Section | Purpose |
|---------|---------|
| `contract_version` | Bump on incompatible profile/config shape |
| `id` | Same `tool_id` as discovery |
| `title` | Human-readable name |
| `endpoint_types` | Optional allowed `type` values for source/target (domain-specific) |
| `connection` | Optional `source` / `target` arrays of field descriptors |
| `profile_fields` | Top-level keys for saved profile / create body |
| `options` | Metadata + **`defaults`** + optional `hints` / `enum` |
| `operations` | Gateway routes (method, path, purpose) |
| `minimal_config_example` | Valid JSON for create (name may differ: `minimal_profile_example` in job-based tools) |
| `worker_limits_env` | Optional env var **names** for worker tuning |

### B.5 Field descriptor

| Key | Meaning |
|-----|---------|
| `key` | JSON property name |
| `kind` | `string`, `int`, `bool`, `object`, `string[]`, `connection`, … |
| `required` | Boolean |
| `default` | For new configs |
| `secret` | `true` → UI/clients redact |
| `enum` | Allowed values |
| `hint` | Short help |

### B.6 Options block (example)

```json
"options": {
  "booleans": ["incremental_sync"],
  "integers": ["parallelism"],
  "strings": ["mode"],
  "enum": { "on_conflict": ["fail", "overwrite", "skip"] },
  "defaults": { "incremental_sync": true, "parallelism": 4 },
  "hints": { "incremental_sync": "Disable for one-shot full transfer." }
}
```

Clients **merge `options.defaults`** when creating a new config.

### B.7 Operations block (example)

```json
{ "method": "POST", "path": "/api/v1/my_tool/configs", "purpose": "create saved config" }
```

Paths are gateway-relative. Clients should prefer `operations` over hardcoded routes.

### B.8 Full minimal contract (example)

```json
{
  "contract_version": "1",
  "id": "my_tool",
  "title": "Example tool",
  "connection": {
    "source": [
      { "key": "host", "kind": "string", "required": true },
      { "key": "port", "kind": "string", "required": false, "default": "8080" },
      { "key": "token", "kind": "string", "required": false, "secret": true }
    ],
    "target": []
  },
  "profile_fields": [
    { "key": "name", "kind": "string", "required": true },
    { "key": "source", "kind": "connection", "required": true },
    { "key": "target", "kind": "connection", "required": true },
    { "key": "options", "kind": "object", "required": false, "default": {} }
  ],
  "options": {
    "booleans": ["flag_a"],
    "defaults": { "flag_a": false }
  },
  "operations": [
    { "method": "GET", "path": "/api/v1/my_tool/contract", "purpose": "this document" },
    { "method": "POST", "path": "/api/v1/my_tool/configs", "purpose": "create config" },
    { "method": "POST", "path": "/api/v1/my_tool/configs/{id}/run", "purpose": "start job" }
  ],
  "minimal_config_example": {
    "name": "demo",
    "source": { "host": "source.example", "port": "8080" },
    "target": { "host": "target.example", "port": "8080" },
    "options": { "flag_a": false }
  }
}
```

Adapt naming (`configs` vs `profiles`, `run` vs `execute`) to your domain; keep the **structure**.

---

## 4. Combined client workflow

1. `GET /{api_prefix}/tools/discover` → list tools; filter `enabled`.
2. `GET {discover_path}` → optional version/limits/errors.
3. `GET {contract_path}` → build UI or agent payload; check `contract_version`.
4. Create config: `minimal_config_example` + `options.defaults` + user input → `POST` from `operations`.
5. Run / poll / stop using remaining `operations` entries.

Clients do **not** hardcode tool lists, worker URLs, flag names, or route paths.

---

## 5. Versioning

| Artifact | When to bump |
|----------|----------------|
| `discover_version` | Catalog or tool-discover field breaking change |
| `contract_version` | Required field rename/remove, type change, incompatible profile shape |

| Change | Contract version |
|--------|------------------|
| New optional field or option | Unchanged |
| Rename/remove required field | Bump |

**Drift:** If worker exposes option lists in `/v1/capability`, **gateway contract wins** for clients.

---

## 6. Adoption checklists

### Gateway

- [ ] Catalog discover with `enabled`, `discover_path`, `contract_path` per tool
- [ ] Tool discover merges `enabled` + worker `/v1/discover` when configured
- [ ] Single module owns all contracts (`contract_version`, `id` aligned with catalog)
- [ ] `GET …/{tool_id}/contract` complete; example validates against handlers
- [ ] Optional: agent/MCP read-only tools for catalog discover and `{tool_id}` contract

### Worker

- [ ] `GET /health`
- [ ] `GET /v1/discover` with `gateway_contract` pointer; runtime only
- [ ] No canonical contract on worker (unless gateway is pass-through)
- [ ] No secrets in discover

### Client

- [ ] Bootstrap: discover → contract only (plus URL + auth)
- [ ] Apply `options.defaults`; redact `secret: true`
- [ ] Handle `enabled: false` and stale catalog (re-fetch on session start)

---

## 7. Anti-patterns

- Secrets or credentials in discover or contract
- Full schema only on worker (clients must know worker URL)
- Mismatched `tool_id` across catalog, URLs, contract, worker
- Using discover alone without contract before create/run
- Breaking config shape without bumping `contract_version`
- Duplicating full contract on gateway and worker

---

## 8. Mapping to your product

Choose `{api_prefix}` and segment names to match your API style. Keep the **roles**:

| Role | Responsibility |
|------|----------------|
| Catalog discover | List tools + pointers |
| Tool discover | Enabled + worker runtime |
| Contract | Schema, defaults, operations |
| Worker discover | Version, limits, `gateway_contract` |
| Worker execute | Jobs/runs (outside this doc) |

Job-based tools use `profiles` + `jobs`; config-only tools may use `configs` + `run` — document in `operations`.

---

*Internal draft — copy into any repo implementing a gateway + optional workers. Supersedes split discovery/contract drafts in this repository.*
