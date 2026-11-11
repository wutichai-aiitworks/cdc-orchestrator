# Paste into Claude Code (or another agent) in the target repo

Apply the deployment-mode pattern from
`/Users/wutichai/workspace/hyper/cdc-orchestrator/docs/patterns/deployment-mode/SKILL.md`
(templates in `templates/`) to this repo.

- Variable name: `APP_MODE` (keep this exact name). Values: Production (default) | Maintenance | Enhanced.
- The old environment variable to retire (read as the only fallback, warn at startup): `<e.g. APP_ENV>`. Delete any other mode variables outright.
- Hardening flags in this repo that Production must default to true: `<list, or "find them">`.
- UI: `<path to the Nuxt/React app>`; ribbon on every page, header controls moved clear.

Before editing: list every place the old variables are read (code, compose, .env*, scripts, docs),
and every live `.env` that would become Production by default — tell me before changing the default.
After: backend + UI tests with the same cases, build, browser check per mode (SKILL.md "Verification").
