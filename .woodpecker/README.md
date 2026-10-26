# .woodpecker/

Woodpecker builds **one workflow per `.yml` file** in this directory, all running in
parallel unless a file declares `depends_on`. Workflow names are the filenames without
the extension. Subdirectories are ignored.

Because this directory exists, a root `.woodpecker.yml` would no longer be read — the
old single-file config was folded into the workflows below.

| Workflow | Events | Depends on | Does |
|----------|--------|-----------|------|
| `test` | pull_request, push, manual | — | `cargo fmt --check`, `test --locked`, `clippy -D warnings` for the root crate, then the standalone `mcp/` crate |
| `webui` | pull_request, push, manual | — | `kafbat-webui`: `npm ci`, `openapi:check`, `typecheck`, `build` |
| `build` | pull_request, push, manual | — | Builds all four images multi-arch **without pushing**, so pull requests catch broken Dockerfiles |
| `sonar` | push, manual | — | `sonar-scanner` against `sonar-project.properties`, waits for the quality gate |
| `main-ci` | push, manual | `test`, `webui`, `build`, `sonar` | Builds and pushes all four images multi-arch, then notifies Discord |

`depends_on` only works at **workflow** level. A step in one file cannot depend on a step
in another file — Woodpecker rejects that as an unknown dependency.

## Images

`build` and `main-ci` cover the same four images from the same Dockerfile/context
pairs. `build` discards the result (`--output type=cacheonly`, since a multi-platform
result cannot be loaded into the local image store); `main-ci` pushes
`linux/amd64` + `linux/arm64`, tagged `latest` and `<branch>-<short-sha>`:

| Image | Dockerfile | Build context |
|-------|-----------|---------------|
| `$REGISTRY_NAMESPACE/hyper-cdc-orchestrator` | `docker/Dockerfile` | repo root |
| `$REGISTRY_NAMESPACE/hyper-cdc-connect` | `docker/connect/Dockerfile` | repo root |
| `$REGISTRY_NAMESPACE/hyper-cdc-webui` | `kafbat-webui/Dockerfile` | `kafbat-webui` |
| `$REGISTRY_NAMESPACE/hyper-cdc-mcp` | `mcp/Dockerfile` | `mcp` |

The orchestrator image needs the repo root as its context because `docker/Dockerfile`
copies `Cargo.toml`, `Cargo.lock`, `build.rs`, `proto/`, and `src/`. The Connect image
also uses the repo root so it can compile the vendored `kafka-connect-jdbc/` fork.

## Required secrets

| Secret | Used by |
|--------|---------|
| `DOCKERHUB_USERNAME` | `main-ci` |
| `DOCKERHUB_TOKEN` | `main-ci` |
| `REGISTRY_NAMESPACE` | `main-ci` (image name prefix) |
| `DISCORD_WEBHOOK` | `main-ci` notification step |
| `SONAR_TOKEN` | `sonar` (token for `https://sonarqube.b-quik-connect.com`, project `hyper-cdc-orchestrator`) |

`test`, `webui`, and `build` need no secrets, so they run safely on pull requests
from forks. `sonar` runs on push and manual only.

## Notes

- Rust is pinned to `1.97-bookworm` here and in every Dockerfile, matching local
  development. `cargo fmt` and `clippy` results drift between rustfmt versions, so keep
  these in sync when bumping.
- Shell expansions in `commands:` must be written `$${VAR}` so Woodpecker does not
  substitute them before the step runs.
- Building four multi-arch images means two Rust release builds under arm64 emulation,
  so both workflows are slow. On a push, `build` and `main-ci` both build — `main-ci`
  runs after `build` and reuses the buildx cache only if Woodpecker schedules both on
  the same agent. If they land on different agents, that is a full rebuild. Options:
  restrict `build` to `pull_request` only (and drop it from `main-ci`'s `depends_on`,
  since `main-ci` builds the same images itself), add a shared registry cache
  (`--cache-to`/`--cache-from`), or use native arm64 agents.
