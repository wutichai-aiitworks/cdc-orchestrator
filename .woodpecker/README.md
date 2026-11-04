# .woodpecker/

One workflow: `main-ci.yml`. Woodpecker builds **one workflow per `.yml` file** in
this directory, so extra files here would show up as separate pipelines.

| Step | Depends on | Does |
|------|-----------|------|
| `test-orchestrator` | — | `cargo fmt --check`, `test --locked`, `clippy -D warnings` |
| `test-mcp` | `test-orchestrator` | same checks in standalone `mcp/` |
| `webui` | — (parallel with rust) | `cdc-native-ui`: `npm ci`, `openapi:check`, `typecheck`, `build` |
| `sonar` | rust tests | `sonar-scanner` against `sonar-project.properties`, waits for the quality gate |
| `build-and-push` | `sonar`, `webui` | Builds and pushes all four images multi-arch |
| `notify-discord` | `build-and-push` (success) | Discord embed with image tags |

Runs on push / manual (not `main`) and on `v*` tags.

## Images

`linux/amd64` + `linux/arm64`, tagged `latest` plus the version:

| Trigger | Version tag |
|---------|-------------|
| `v*` git tag | the tag (`v1.2.3`) |
| branch push | `<branch-slug>-<short-sha>` |
| `main` or empty branch | `sha-<short-sha>` |

| Image | Dockerfile | Build context |
|-------|-----------|---------------|
| `$REGISTRY_NAMESPACE/cdc-orchestrator` | `docker/Dockerfile` | repo root |
| `$REGISTRY_NAMESPACE/cdc-connect` | `docker/connect/Dockerfile` | repo root |
| `$REGISTRY_NAMESPACE/cdc-webui` | `cdc-native-ui/Dockerfile` | `cdc-native-ui` |
| `$REGISTRY_NAMESPACE/cdc-mcp` | `mcp/Dockerfile` | `mcp` |

The orchestrator image needs the repo root as its context because `docker/Dockerfile`
copies `Cargo.toml`, `Cargo.lock`, `build.rs`, `proto/`, and `src/`. The Connect image
also uses the repo root so it can compile the vendored `kafka-connect-jdbc/` fork.

## Required secrets

| Secret | Used by |
|--------|---------|
| `DOCKERHUB_USERNAME` | `build-and-push` |
| `DOCKERHUB_TOKEN` | `build-and-push` |
| `REGISTRY_NAMESPACE` | `build-and-push` (image name prefix) |
| `DISCORD_WEBHOOK` | `notify-discord` |
| `SONAR_TOKEN` | `sonar` (token for `https://sonarqube.b-quik-connect.com`, project `hyper-cdc-orchestrator`) |

All steps need the secrets above (no pull-request path).

## Notes

- Rust is pinned to `1.97-bookworm` here and in every Dockerfile, matching local
  development. `cargo fmt` and `clippy` results drift between rustfmt versions, so keep
  these in sync when bumping.
- Shell expansions in `commands:` must be written `$${VAR}` so Woodpecker does not
  substitute them before the step runs.
- Four multi-arch images means two Rust release builds under arm64 emulation, so
  publish is slow. A shared registry cache (`--cache-to` / `--cache-from`) or native
  arm64 agents would help if that becomes a problem.
