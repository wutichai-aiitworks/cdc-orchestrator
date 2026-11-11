# ADR NNNN — APP_MODE: one deployment-mode variable

Status: Accepted — YYYY-MM-DD

## Context

<Which variables described the deployment before (e.g. APP_ENV=dev|prod, a UI-only mode flag), and what each controlled.>
The team names deployments **Production**, **Maintenance** (stable, fixes only) and **Enhanced**
(new features in development), not dev/sit/uat/prod.

## Decision

- `APP_MODE=Production|Maintenance|Enhanced` is the only variable, read by every process (compose
  passes it to each). Default when unset: `Production`. Case-insensitive; common spellings accepted.
- UI: no ribbon in Production; 45° corner ribbon, amber for Maintenance and violet for Enhanced. The header
  controls move clear while it shows.
- Hardening (`APP_REQUIRE_AUTH`, `APP_REQUIRE_PERSISTENCE`, `APP_STRICT_PERSISTENCE`) defaults to true
  only in Production; explicit flags override in any mode. Empty values mean "default".
- Retired variables are still read when `APP_MODE` is unset (`APP_ENV=dev` → Enhanced), with a
  startup warning; removed in <version>.

## Consequences

- Without `APP_MODE`, a deployment needs a token and a database; dev env files set `Enhanced`.
- Rules live in `<backend file>` and `<ui util>`, kept identical and tested on both sides.
- Pattern: hyper cdc-orchestrator `docs/patterns/deployment-mode/`.
