# ADR 0011 — APP_MODE: one deployment-mode variable (replaces ORCH_ENV)

Status: Accepted — 2026-11-07

## Context

`ORCH_ENV=dev|prod` described the deployment and also switched auth/persistence hardening on. The
web UI needs to show the deployment mode too (a corner ribbon), and the team
names deployments **Production**, **Maintenance** (stable, fixes only) and **Enhanced** (new
features in development) — not dev/sit/uat/prod.

## Decision

- `APP_MODE=Production|Maintenance|Enhanced` is the only variable, read by the orchestrator and
  the web UI (compose passes it to both). Default when unset: `Production`. Common spellings are
  accepted (`prod`, `Maintainance`, `maint`, `enhance`, `enhancement`); case does not matter.
- Web UI: Production shows no ribbon; Maintenance an amber and Enhanced a violet 45° corner ribbon
  on every page. While a ribbon shows, the header's theme button and avatar move clear of it.
- Hardening: `Production` defaults `ORCH_REQUIRE_AUTH`, `ORCH_REQUIRE_PERSISTENCE` and
  `ORCH_STRICT_PERSISTENCE` to true (what `ORCH_ENV=prod` did). Maintenance and Enhanced default them
  to false; set them `true` explicitly when such a deployment is reachable by others.
  `production-preflight.sh` enforces this.
- An empty `ORCH_REQUIRE_*` (compose's `${VAR:-}`) means "use the default", not "false".
- Migration: `ORCH_ENV` is retired but still read when `APP_MODE` is unset (`dev` → Enhanced,
  `prod` → Production), with a startup warning. No other mode variable is read.

## Consequences

- Rules live in `src/config.rs` (`AppMode`) and `cdc-native-ui/utils/appMode.ts`, kept
  identical and tested on both sides.
- Because the default is Production, a deployment without `APP_MODE` needs a strong token and
  rqlite; development `.env` files set `APP_MODE=Enhanced`.
- Reusable pattern for other repos (generic names, templates, gotchas): `docs/patterns/deployment-mode/`.
