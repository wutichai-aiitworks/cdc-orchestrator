# ADR 0002 — Standalone login (`AUTH_MODE=local`) for the native WebUI

Status: Accepted
Owner: Wutichai
Last updated: 2026-11-01

## Context

The native WebUI (Nuxt, `:4320`) had two auth modes: `none` (open) and `sso`
(SuperTokens gateway / Authelia). Installs without an SSO provider needed a
simple login without giving up the session-guarded `/api/cdc/*` BFF.

The WebUI is a backend-for-frontend: the browser only talks to Nitro, which
proxies to the orchestrator with `ORCHESTRATOR_TOKEN`. A UI login therefore only
protects the UI path; the orchestrator API (`:9788`) has its own gate
(`ORCH_API_TOKEN`) and is open when that is unset.

Two near-duplicate UI trees exist: `kafbat-webui/` (tracked) and
`cdc-native-ui/` (newer; what compose and `.woodpecker/webui.yml` build). Both are
kept for now.

## Threat model

Internal network / VPN only, one shared operator account. Not internet-exposed,
not shipped to customers. Browsers reach `:4320` directly — no reverse proxy.

## Decision

1. **Third auth mode.** `AUTH_MODE = none | local | sso`. `local` shows a
   username/password form at `/auth/login`.
2. **Env-only credentials.** `ADMIN_USERNAME` / `ADMIN_PASSWORD`, default
   `admin` / `admin`. No stored state, no in-UI password change, no forced change
   on first login, no hashed variant. Production startup logs a warning while the
   default is active.
3. **Sessions.** Same sealed `cdc-session` cookie as SSO (8 h, `httpOnly`,
   `sameSite=strict`). `SESSION_SECRET` is optional in `local` mode — if unset or
   shorter than 32 chars, a per-process random secret is used (sessions reset on
   restart). `sso` still requires an explicit secret.
4. **Brute-force limit.** 10 failed logins per client IP per 5 minutes → 429.
   The client IP is the **socket address**; `X-Forwarded-For` is honoured only
   with `TRUST_PROXY=true` (otherwise a client could rotate the header to dodge
   the lockout). Constant-time comparison of both fields.
5. **No open orchestrator.** `docker/install.sh` writes a random 48-char
   `ORCH_API_TOKEN` into `docker/.env` when none is set. Compose already passes it
   to the orchestrator, the WebUI (`ORCHESTRATOR_TOKEN`) and `mcp-cdc`
   (`CDC_ORCHESTRATOR_TOKEN`). This closes the container-to-orchestrator bypass on
   the compose network.
6. **UI.** Header shows a circular avatar (`.avatar`) opening a menu
   (`.menu-panel` / `.menu-item`, shared classes in `assets/css/main.css`) with the
   user and **Sign out**. Visible whenever auth is not `none`.

## Consequences

- Once `install.sh` generates a token, host-side callers of `:9788`
  (`scripts/soak-test.sh`, ad-hoc `curl`, other services) need
  `Authorization: Bearer $ORCH_API_TOKEN` from `docker/.env`.
- Lockout counters are in memory per process — fine for a single UI container,
  becomes per-replica if the UI scales out.
- Every auth change must be made in **both** UI trees. Run
  `scripts/check-native-ui-parity.sh` before finishing.

## Revisit when

The deployment model changes — internet exposure (Cloudflare tunnel / public
proxy), customer installs, or multiple operators. Then reconsider: no default
credentials in production, forced password change or install-time random
password, `ADMIN_PASSWORD_HASH`, `TRUST_PROXY=cloudflare` (`CF-Connecting-IP`),
`SESSION_COOKIE_SECURE=true`, and per-user accounts.

## Files

| Concern | Path (same in both UI trees) |
|---|---|
| Mode switch | `utils/authMode.ts`, `nuxt.config.ts` |
| Login API | `server/api/auth/login.post.ts` |
| Session | `server/utils/sessionConfig.ts`, `server/middleware/admin-auth.ts` |
| me / logout | `server/api/auth/me.get.ts`, `server/api/auth/logout.post.ts` |
| Startup checks | `server/plugins/security-startup.ts` |
| Client | `composables/useAuth.ts`, `plugins/auth-guard.client.ts`, `pages/auth/login.vue` |
| Avatar menu | `components/AppUserMenu.vue`, `layouts/default.vue`, `assets/css/main.css` |
| Deploy | `docker/docker-compose.yaml`, `docker/.env.example`, `docker/install.sh` |
