# Agent prompt — apply the break-glass admin pattern

Paste into the target repo's agent session:

> Apply the break-glass admin pattern from
> `/Users/wutichai/workspace/hyper/cdc-orchestrator/docs/patterns/break-glass-admin/SKILL.md`
> to this repo's web UI. Read SKILL.md fully first, then the templates next to it.
>
> 1. Find the UI's auth modes. If `AUTH_MODE=local` (env-only `ADMIN_USERNAME`/`ADMIN_PASSWORD`,
>    constant-time compare, lockout, sealed session) doesn't exist, stop and tell me — that is a
>    prerequisite, not part of this change.
> 2. Make the session cookie name configurable (`SESSION_COOKIE_NAME`) and route every session
>    read/write through one helper.
> 3. Add `start-ui.mjs` (adapt `adminOverrides` to this repo's env names), switch the Dockerfile
>    entrypoint, `EXPOSE 3000 3001`, and make the healthcheck probe both ports.
> 4. In compose, add `127.0.0.1:${<APP>_ADMIN_PORT:-4321}:3001` and `ADMIN_PORT`/`ADMIN_SESSION_SECRET`
>    to the existing UI service. No new service. If anything else defaults to host port 4321, move it.
> 5. Document: ADR (or amendment), `.env.example`, `AGENTS.md` rule + "SSO is down" prompt.
> 6. Run the "Verification" list from SKILL.md and report each item. Don't deploy or commit unless asked.
