---
name: deployment-mode
description: Add one deployment-mode environment variable (APP_MODE = Production | Maintenance | Enhanced) to a service and its web UI, with a 45° corner ribbon (amber Maintenance, violet Enhanced, none in Production), Production-by-default security hardening, migration from older env variables, and preflight checks. Use when a repo needs an environment/mode/release flag, a "maintenance" or "beta" banner or ribbon, or wants to merge ORCH_ENV/APP_ENV/NODE_ENV-style variables into one.
---

# Deployment mode: one variable, three values, one ribbon

Reference implementation: **hyper cdc-orchestrator** (`APP_MODE`). See ADR 0011 there
(`docs/adr/0011-app-mode.md`), `src/config.rs` (`AppMode`) and `cdc-native-ui/utils/appMode.ts`.
The templates in this folder are the same code with names made generic.

## The contract (keep it identical in every repo)

| Value | Meaning | UI | Hardening default |
|---|---|---|---|
| `Production` (**default when unset**) | Live system in normal service | no ribbon | **on** |
| `Maintenance` | Stable release, fixes only | amber ribbon "MAINTENANCE" | off |
| `Enhanced` | Release with new features in development | violet ribbon "ENHANCED" | off |

- **One variable, same name everywhere:** `APP_MODE`. Keep this exact name in every repo, so operators and scripts never have to remember a per-product prefix. Use `<PREFIX>_MODE` only if `APP_MODE` already means something else in that repo. Every process reads the same variable: backend, UI server and workers. Compose passes it to each container. Never add a second "environment" variable next to it.
- **Parsing:** case-insensitive. Accept `prod` and `production`; `maintenance`, `maintainance` (a common misspelling) and `maint`; `enhanced`, `enhance`, `enhancement` and `enhancements`. Anything else is unknown, and unknown falls back to the default. `dev`, `sit` and `uat` are **not** modes.
- **Hardening:** require auth, require persistence and strict persistence default to `mode == Production`.
  - Explicit flags override the default in any mode. They keep the repo's own prefix (`ORCH_REQUIRE_AUTH` in the reference; `APP_REQUIRE_AUTH` in the templates).
  - A Maintenance or Enhanced deployment that other people can reach must set them to `true`.
- **Empty means unset.** `VAR=` and compose's `${VAR:-}` give an empty string, and it must mean "use the default", never `false`. The reference repo had this bug: compose passed `ORCH_REQUIRE_AUTH: ${ORCH_REQUIRE_AUTH:-}`, which silently turned auth off even with prod set.
- **Migration:** if `APP_MODE` is unset, read the one retired environment variable (`ORCH_ENV` in the reference, `APP_ENV` in the templates): `dev` maps to Enhanced, `prod` to Production. Log one startup warning while it is still set. Read no other old mode variables, and remove this fallback one release later. Any short-lived mode variables that were never deployed are deleted outright, not kept as fallbacks.
- **Runtime, not build time.** Read the mode when a request arrives or at process start, so changing it needs a restart, not a rebuild.

## Files to add or change (checklist)

Backend (pick your language):
- [ ] Mode enum and parser, plus `resolve(new, legacy...)` and `hardened_by_default()`. Templates: `templates/rust/mode.rs`, `templates/python/mode.py`, `templates/go/mode.go`.
- [ ] Wire it into config. The hardening flags should be `env_truthy(FLAG).unwrap_or(mode.hardened_by_default())`, with `env_truthy` treating an empty value as unset.
- [ ] Put `mode=` in the startup log line, and add the warning for the retired variable.
- [ ] Fail-fast messages name the flag that caused them: "`<PREFIX>_REQUIRE_AUTH=true` requires …".
- [ ] Unit tests: spellings, resolve order, and that only Production hardens by default. Template: `templates/rust/mode_tests.rs`.

Web UI (Nuxt 3 templates; for other frameworks see "Porting the UI" below):
- [ ] `utils/<prefix>Mode.ts`: the same parser and rules as the backend, plus labels. No Tailwind classes in this file.
- [ ] `server/api/ui/mode.get.ts`: a public endpoint (the login page shows the ribbon too) that reads `process.env` on each request.
- [ ] `composables/use<Prefix>Mode.ts`: `useState` holding the mode, and `ribbon` (null in Production).
- [ ] `components/AppModeRibbon.vue`: the ribbon. Its colour classes live **here**.
- [ ] `app.vue`: `await useFetch('/api/ui/mode')` during server rendering, then render `<AppModeRibbon />` outside `<NuxtLayout>`.
- [ ] The header's right-hand controls: `:class="ribbon ? 'mr-16 md:mr-14' : ''"`, so the avatar and theme button clear the corner.
- [ ] A unit test for the TS rules, with the same cases as the backend test. Template: `templates/tests/mode.test.ts`.

Ops:
- [ ] Compose: `APP_MODE: ${APP_MODE:-Production}` on every service that reads it. Template: `templates/ops/compose-snippet.yaml`.
- [ ] `.env.example` gets `Enhanced`. `.env.production.example` gets `Production` and keeps the three flags explicitly `true`. Template: `templates/ops/env-snippet.env`.
- [ ] Preflight script: the mode must be valid. Each hardening flag must be `true`, or unset with mode Production. Warn when the retired variable is present. Template: `templates/ops/preflight-snippet.sh`.
- [ ] Docs: the environment-variable table, install guide, README, and an ADR. Template: `templates/ops/ADR-template.md`.
- [ ] **Before changing the default, check every live `.env`.** A deployment with no mode set becomes Production and refuses to start without a token or database. Set `Enhanced` in dev env files first.

## Ribbon spec (so every product looks the same)

- Container: `fixed right-0 top-0 z-[70] h-28 w-28 overflow-hidden pointer-events-none`, with `role="status"` and `aria-label="<Label> mode"`.
- Band: `absolute right-[-46px] top-[22px] w-[180px] rotate-45 border-y py-1 text-center text-[11px] font-bold uppercase tracking-[0.18em] text-white`.
- Stripes: `background-image: repeating-linear-gradient(-45deg, rgb(0 0 0 / .08) 0 6px, transparent 6px 12px)`.
- Colours:
  - Maintenance: `bg-amber-500 border-amber-700/30` (#f59e0b)
  - Enhanced: `bg-violet-600 border-violet-800/30` (#7c3aed)
  - Production: no element at all.
- Tooltip (`title`): `APP_MODE=Maintenance — stable release, fixes only`, or `APP_MODE=Enhanced — release with new features in development`.
- Flat design: no shadow.

## Gotchas found while building the reference

1. **Tailwind purges classes it doesn't scan.** The reference `tailwind.config` covers `components/`, `pages/`, `layouts/`, `composables/` and `plugins/`, but not `utils/`. Classes written only in a `utils/*.ts` file are missing from the CSS, and nothing reports it. Keep colour classes in the `.vue` component, or add the folder to `content`. Check the build with `grep bg-violet-600 .output/public/_nuxt/*.css`.
2. **Tailwind opacity modifiers don't work on CSS-variable colours.** `bg-primary/10` produces nothing when `primary` is `var(--primary)`. Use palette colours (`amber-700/30`) as above.
3. **Nuxt `runtimeConfig.public` is fixed at build time.** Read `process.env` in a server route instead, or pass `NUXT_PUBLIC_*` at runtime. `useFetch` with a key, called in `app.vue`, runs during server rendering, so the ribbon is in the first HTML and doesn't flash in.
4. **Nuxt's error page replaces `app.vue`**, so a 404 or 500 page has no ribbon. That's acceptable; just don't test the ribbon on a route that doesn't exist. Example: with `AUTH_MODE=none` there is no `/login` route, so test `/`.
5. **Two copies of the rules can drift.** Keep the backend and UI parsers identical and give both the same test cases. If the repo has a mirrored UI (cdc-native-ui and kafbat-webui in the reference), copy the files and run the parity check.
6. **Don't infer security from a release label without a way to override it.** Production hardening by default is safe. Maintenance and Enhanced must still be able to turn it on, and preflight must require them to when they're used for real.
7. **Preview servers may refuse open auth** (`AUTH_MODE=none is not allowed in production`). For a local screenshot only, set `ALLOW_OPEN_AUTH=true` on 127.0.0.1. Never put it in a config file.

## Verification (copy into the PR description)

- Backend tests: spellings, resolve order, hardening default.
- UI unit tests: the same cases.
- Build, then grep the built CSS for `bg-amber-500` and `bg-violet-600`.
- Start the built UI with each of `APP_MODE=Production`, `Maintainance`, `enhanced` and unset. Check:
  - the `GET /api/ui/mode` response;
  - the ribbon count: 0 for Production and unset, 1 otherwise;
  - the ribbon colour: rgb(245,158,11) for Maintenance, rgb(124,58,237) for Enhanced;
  - the header's right edge moves left only while a ribbon shows. Reference, at 1440 px: 1416 without a ribbon, 1360 with one.
- Only the retired `ENV=dev` set: the endpoint returns `enhanced`, and the backend logs the warning.
- Production with no token: the backend refuses to start, and the message names `<PREFIX>_REQUIRE_AUTH`.

## Porting the UI (non-Nuxt)

- **React/Next:** the same endpoint, read on the server (`getServerSideProps`, a route handler, or a root layout server component). Render `<ModeRibbon/>` in the root layout. Same classes.
- **Plain SPA behind nginx:** serve `/api/ui/mode` from the backend, or template a `<meta name="app-mode">` into `index.html` at container start with `envsubst`. Fetching it on the client causes a brief flash; that's acceptable.
- **Server-rendered (Jinja, Go templates):** pass `mode` into the base template, and render the ribbon markup when it isn't `production`.
