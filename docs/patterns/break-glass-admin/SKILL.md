---
name: break-glass-admin
description: Add a break-glass admin login to a web UI whose normal login is SSO (SuperTokens, Authelia, OIDC). The same container runs a second process of the same build on a loopback-only port (default 4321) with a fixed local username/password, its own session cookie and secret, and a small supervisor that stops both if either dies. Use when a repo's UI must stay reachable while the SSO provider is down, or when someone asks for an "admin port", "local fallback login", "emergency access" or "break-glass" channel.
---

# Break-glass admin: second listener, same container, loopback only

Reference implementation: **hyper cdc-orchestrator** native WebUI (`cdc-native-ui/`).
Decision record: `docs/adr/0002-native-ui-local-auth.md` → "Amendment 2026-11-11".
Code: `cdc-native-ui/start-ui.mjs`, `cdc-native-ui/Dockerfile`,
`cdc-native-ui/server/utils/sessionConfig.ts`, `docker/docker-compose.yaml` (`cdc-native-ui`).
The templates in this folder are the same code with the names made generic.

## The contract (keep it identical in every repo)

| | Main listener | Break-glass listener |
|---|---|---|
| Container port | `3000` (`PORT`) | `3001` (`ADMIN_PORT`; empty = off) |
| Host publish | `${<APP>_HOST_PORT:-4320}:3000` (as today) | **`127.0.0.1:${<APP>_ADMIN_PORT:-4321}:3001`** |
| Auth | `AUTH_MODE` as configured (`sso` in prod) | **`local`, forced** by the launcher, not by env |
| Credentials | SSO provider | `ADMIN_USERNAME` / `ADMIN_PASSWORD` (shared env) |
| Session cookie | `<app>-session` | **`<app>-admin-session`** |
| Session secret | `SESSION_SECRET` | **`ADMIN_SESSION_SECRET`**, empty = random per process |
| `TRUST_PROXY` | as configured | **`false`, forced** |

Rules:

- **Loopback only, not configurable.** Write `127.0.0.1:` literally in compose. Only the
  host port is a variable. Remote operators use an SSH tunnel:
  `ssh -L 4321:127.0.0.1:4321 <host>`, then open `http://127.0.0.1:4321`. Host SSH is the
  outer auth layer.
- **Never "fall back to local when SSO fails" on the main port.** That turns an SSO outage
  into an auth downgrade on the port everyone can reach. The fallback is a *different port*
  that only the host can reach.
- **Same build, same container, two processes.** The launcher starts both from the same
  server bundle with different env. No auth code branches on "which port am I".
- **Own cookie name and own secret.** Browsers scope cookies by host, **not port**.
  `127.0.0.1:4320` and `127.0.0.1:4321` share a cookie jar. With the same cookie name and
  secret, a local login on `:4321` would be a valid session on `:4320`.
- **Supervisor semantics.** If either child exits, the launcher stops the other and exits
  with that child's code, so `restart: unless-stopped` brings both back. It forwards
  SIGTERM/SIGINT. It prefixes logs `[ui]` / `[admin]`.
- **Healthcheck covers both ports** when `ADMIN_PORT` is set.
- **Always on.** No compose profile opt-in: the channel has to be running *before* SSO breaks.
- **Port 4321 is reserved** for this channel across repos. Move anything else that defaults
  there (the reference moved Kafbat UI to 28080).

## Files to add or change (checklist)

App (Nuxt/Nitro templates; other stacks under "Porting" below):
- [ ] `start-ui.mjs`: launcher/supervisor. Template: `templates/node/start-ui.mjs`. Edit the
      `adminOverrides` keys to match your env names.
- [ ] Session config reads the cookie name from `SESSION_COOKIE_NAME` (default `<app>-session`)
      and validates it (`^[A-Za-z0-9_-]{1,64}$`). Template: `templates/node/sessionConfig.ts`.
- [ ] Runtime config exposes it: `sessionCookieName: env("SESSION_COOKIE_NAME") || "<app>-session"`.
- [ ] `AUTH_MODE=local` already exists, with: env-only credentials, constant-time compare,
      a lockout (10 fails / 5 min / IP), and a random session secret when none is set. If it
      doesn't, build that first (reference: ADR 0002, decisions 1–4).
- [ ] Every `useSession` / cookie read goes through the one session-config helper. Grep for
      the literal cookie name and remove the hard-coded copies.

Image (`templates/docker/Dockerfile.snippet`):
- [ ] `COPY start-ui.mjs` (plus `map-env.mjs` if you have one), `ENTRYPOINT ["node", "/app/start-ui.mjs"]`.
- [ ] `EXPOSE 3000 3001`.
- [ ] `HEALTHCHECK` probes `PORT` and `ADMIN_PORT`.
- [ ] Inside the container the processes listen on `0.0.0.0` (`HOST=0.0.0.0`). Loopback
      restriction happens at the **host publish**, not inside the container.

Compose (`templates/docker/compose.snippet.yaml`):
- [ ] Second `ports:` entry `127.0.0.1:${<APP>_ADMIN_PORT:-4321}:3001`.
- [ ] `ADMIN_PORT: "3001"` and `ADMIN_SESSION_SECRET: ${ADMIN_SESSION_SECRET:-}`.
- [ ] `.env.example`: document `<APP>_ADMIN_PORT`, `ADMIN_SESSION_SECRET`, and that
      `ADMIN_PASSWORD` must not stay at the default when `AUTH_MODE=sso`.
- [ ] Installer/start script prints the admin URL. Deploy scripts need no change (same service).

Docs:
- [ ] ADR (or amendment) in the repo: context, the decision table above, rejected options.
- [ ] `AGENTS.md`: one rule line plus a short prompt "SSO is down / break-glass login" →
      tunnel, open `:4321`, log in with `ADMIN_*`; if refused, `docker logs <ui> | grep '\[admin\]'`.

## Rejected options (don't re-propose without a new reason)

| Option | Why not |
|---|---|
| Separate compose service for admin | Works, but one more container per UI to deploy, watch and keep in sync. Was the first cut in the reference, dropped at the owner's request. Fine if a repo prefers isolation over container count. |
| One process, two listeners, auth chosen per request by port | Auth mode is read in middleware, login/me/SSO handlers, startup checks and the client payload. Every one would need to become per-request; one miss mixes SSO and local on the exposed port. |
| Runtime "fallback to local if SSO is unreachable" on the main port | Auth downgrade on the reachable port; an attacker who can break SSO reachability gets a password prompt. |
| Opt-in compose profile for the admin channel | It must already be running when SSO breaks. |
| Publishing on `0.0.0.0` "for convenience" or a configurable bind | Defeats the model. Tunnel instead. |

## Gotchas found while building the reference

- **Cookies ignore ports.** Same host, different port = same cookie jar. This is the whole
  reason for the separate cookie name *and* secret.
- **Env precedence in Nuxt.** `NUXT_*` / `NUXT_PUBLIC_*` win over raw vars. When overriding a
  key for the admin child, delete its `NUXT_<KEY>` and `NUXT_PUBLIC_<KEY>` copies too, or a
  value set on the container leaks through (e.g. `NUXT_PUBLIC_AUTH_MODE=sso`).
- **`undefined` in a spawn env becomes the string `"undefined"`.** Delete keys instead of
  setting them to `undefined`. Empty string means "unset": delete it.
- **Don't pass `ADMIN_SESSION_SECRET` to the main child.** Strip it from the main env so the
  admin secret only exists in the admin process.
- **Lockout keys on the Docker gateway IP** for all loopback clients (the socket address
  seen through the port publish). That's acceptable for one operator and is why
  `TRUST_PROXY` is forced `false` on this listener.
- **`Secure` cookie on plain HTTP.** With `NODE_ENV=production` the cookie may default to
  `Secure`. Set `ADMIN_SESSION_COOKIE_SECURE=false` (the launcher defaults it) since the
  tunnel is `http://127.0.0.1`.
- **Testing the supervisor:** don't `pkill -f <pattern>`; it matches the test shell's own
  command line and kills it. Use `lsof -t -iTCP:<admin port> -sTCP:LISTEN` to find the child.
- **Default credentials.** `admin/admin` on a loopback port is still reachable by anyone with
  a shell on the host. Startup should warn; prod `.env` must set `ADMIN_PASSWORD`.

## Verification (copy into the PR description)

Run the built server through the launcher with `PORT=14320 ADMIN_PORT=14321 AUTH_MODE=sso`
(dummy SSO env) and `ADMIN_USERNAME/PASSWORD` set:

- [ ] Main port: local login rejected (404/401); `/api/...` without session → 401.
- [ ] Admin port: wrong password → 401; right password → 200 and `Set-Cookie: <app>-admin-session`.
- [ ] Admin cookie sent to the main port → 401.
- [ ] Kill the admin child (`kill -9 $(lsof -t -iTCP:14321 -sTCP:LISTEN)`) → launcher exits, main port closed.
- [ ] `kill -TERM <launcher>` → both listeners gone within a few seconds.
- [ ] `docker compose config` shows `host_ip: 127.0.0.1` on the admin port.
- [ ] Container `healthy`; `docker logs` shows `[ui]` and `[admin]` lines.

## Porting (non-Nuxt)

The contract stays the same; only the launcher changes.

- **Any Node server (Express, Fastify, Next standalone):** reuse `start-ui.mjs` and point
  `serverEntry` at your entry file. Drop the `map-env.mjs` hop if you don't have one.
- **Python (uvicorn/gunicorn):** two processes are simplest. Use a tiny supervisor script or
  `honcho`/`supervisord` with "stop all on any exit". Or run two `uvicorn` servers in one
  asyncio loop with separate app instances built from a factory that takes `auth_mode`. Keep
  the instances separate; don't branch on `request.url.port`.
- **Go / Rust:** one binary, two `http.Server` / `axum::serve` tasks, each built from its own
  config struct (`auth_mode`, cookie name, secret). This is safe there because the config is
  an explicit value per router, not global env. Exit the process if either task ends.
- **Behind Kubernetes:** a second container in the same pod (sidecar) with
  `kubectl port-forward` instead of an SSH tunnel; never a Service/Ingress for it.
