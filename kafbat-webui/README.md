# kafbat-webui

Level-3 native Kafka/CDC admin UI for `hyper-cdc-orchestrator` using Nuxt 3 + Tailwind, aligned with `hyper-dataservice-webui` design language.

## Design system and theme

- Uses the same semantic `primary`, `secondary`, `background`, `card`, `muted`,
  `border`, sidebar, brand, and surface tokens as `hyper-dataservice-webui`.
- Primary is locked to `rgb(226, 151, 73)` (`#e29749`).
- Light mode is the default.
- The header toggle switches between light and dark modes.
- Theme preference is persisted under `ds-color-scheme`, matching the reference
  WebUI.
- Shared 40px controls, `rounded-xl` geometry, Inter typography, table, status,
  card, focus-ring, scrollbar, and responsive navigation standards are applied
  across all native CDC pages.

## Purpose

Replace dependency on Kafbat UI screens with a first-party web app that talks directly to orchestrator `/cdc/*` APIs.

## Local development

Docker Compose (from the orchestrator repo root):

```bash
./install.sh --native-ui
```

Native UI: `http://127.0.0.1:4320` (orchestrator at `http://127.0.0.1:9788`).

Hot-reload without images:

```bash
./start.sh --build .env_local
```

## Environment

Create `.env_local` from `.env.example`.

See `.env.example`:

- `APP_NAME`, `ORCHESTRATOR_URL`, `PUBLIC_ORCHESTRATOR_URL`, `ORCHESTRATOR_TOKEN`
- `AUTH_MODE`, `SESSION_SECRET`, `SSO_APP_ID`, `SSO_LOGIN_URL`, `SSO_CALLBACK_URL`, `SSO_EXCHANGE_URL`, `SSO_APP_SECRET`
- `SSO_EXCHANGE_DEBUG=true` to log `/auth/custom-sso/exchange` request and response (default off). Ad-hoc, no restart: `POST /api/auth/sso/debug` with `{"enabled":true}` on `:4320`
- Authelia: `SSO_PROVIDER=authelia` plus `AUTHELIA_*`

## Build

```bash
npm run openapi:check
npm run typecheck
npm run build
```

Output: `.output/`

## API reference

The native Scalar reference is available at `/api-docs`; the bundled contract is
served at `/openapi.json`.

`../docs/openapi.yaml` is canonical. Regenerate both checked-in JSON copies after
changing it:

```bash
npm run openapi:generate
```

CI runs `openapi:check` to reject stale bundles.

## Current status

- M1 topic/message/consumer-group observability is implemented.
- M2 connector CRUD, validation, actions, task traces, timeline, and dry-run workflows are implemented.
- Kafbat UI v1.5.0 cluster menu parity: Dashboard, Brokers, Topics, Consumers, Schema Registry, Kafka Connect, KSQL DB, and ACL, including produce, CSV export, offset reset, and schema/ACL CRUD.
- Scalar API reference and downloadable OpenAPI JSON are implemented.
