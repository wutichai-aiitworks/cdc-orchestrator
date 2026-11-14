# .woodpecker/

One workflow: `main-ci.yml`. Woodpecker builds **one workflow per `.yml` file** in
this directory, so extra files here would show up as separate pipelines.

| Step | Depends on | Does |
|------|-----------|------|
| `test-orchestrator` | — | `cargo fmt --check`, `test --locked`, `clippy -D warnings` |
| `test-mcp` | `test-orchestrator` | same checks in standalone `mcp/` |
| `webui` | — | `cdc-native-ui`: `npm ci`, `openapi:check`, `typecheck`, `build` |
| `test-sidecars` | — | `unittest` for `docker/sidecar_common` + every Python sidecar |
| `test-wireguard` | — | `go vet` + `go test` in `docker/wireguard` |
| `test-jdbc-admin` | — | `mvn test` in `docker/jdbc-admin` (in-memory H2) |
| `sonar` | rust tests | `sonar-scanner` against `sonar-project.properties`, waits for the quality gate |
| `build-and-push` | `sonar`, `webui`, all sidecar tests | Builds and pushes all twelve images multi-arch |
| `notify-discord` | every step (success **or** failure) | `scripts/ci/notify-discord.sh`: one Discord embed for the whole run |

Runs on push / manual (not `main`) and on `v*` tags.

## Tags

`linux/amd64` + `linux/arm64`:

| Trigger | Tags pushed |
|---------|-------------|
| `v*` git tag | the tag (`v1.2.3`, no commit id), then `latest` |
| branch push | `<branch-slug>_<short-sha>` only (e.g. `feature-capture_view_31d8e3f`) |
| no branch (detached) | `sha_<short-sha>` only |

The same string is baked into `cdc-webui` as `APP_VERSION` (`--build-arg`) and shown
under the sign-in form when `AUTH_MODE=local`, so the login page names the exact
image tag. Local builds through `docker/compose.inc.sh` derive it from git with the
same rule (override with `CDC_APP_VERSION`); a plain `docker build` shows `dev`.

`latest` moves **only on `v*` tags** and only **after all twelve images built**, by
retagging each image's pushed digest (`buildx imagetools create`). A branch build never
changes what `latest` points at, and a release that fails halfway leaves `latest`
on the previous release.

Each build also carries:

- OCI labels: `title`, `version`, `revision` (commit SHA), `source`, `created`.
- Provenance (`mode=max`) and an SBOM attestation.
- `--pull`, so floating bases (`postgres:18`, `debian:bookworm-slim`, …) pick up
  security patches on every build.
- Registry layer cache at `<image>:buildcache` (Docker Hub supports it with
  `image-manifest=true`). A cache export error does not fail the build.

## Discord notification

Same pattern as `supertokens-gateway`. Every step writes `.ci-status/<key>` (`running` first,
`success` last); `scripts/ci/notify-discord.sh` reads them into a flow block:

```
checkout   ✅
rust       ✅
...
sonar      ✅
build      ❌
  cdc-orchestrator  ✅
  ...
  cdc-pgcopydb      ❌          <- image-cdc-pgcopydb left at "running"
  cdc-pgdump        ⏭️ skipped  <- not reached
  ...
publish    ⏭️ skipped           <- no file: step never ran
```

The per-image lines come from `.ci-status/image-<name>`, written by the build loop for each
entry in `scripts/ci/images.txt`. That file is the single image list for both the build and
the notifier; add a new image there, not in the YAML. Image lines appear only when the build
step ran (a test failure shows just `build ⏭️ skipped`).

Green "cdc-orchestrator pipeline succeeded" or red "… failed". Fields: Project, Branch/Tag,
Triggered by, Images, Tags, Platforms, Commit, Pipeline. The Images/Tags fields say what
actually reached the registry:

| Outcome | Images / Tags |
|---------|---------------|
| publish ✅ | image list + `images-<version>.env` attached; tags pushed |
| build ✅, publish ❌ | all pushed under the version tag; `latest` unchanged |
| build ❌ | failed partway: earlier images may be pushed (`incomplete`) |
| build skipped | not published; version `(not pushed)` |

The version rule lives in `scripts/ci/version.sh` (shared by build and notify, so a failed
run still reports the right version). The curl image runs as non-root, so the payload is
written to `/tmp`, not the workspace.

## Release manifest (`images.env`)

`build-and-push` writes `.ci-status/images.env`, one line per image in the compose
variable format, pinned by digest:

```
CDC_ORCHESTRATOR_IMAGE=<ns>/cdc-orchestrator:v1.2.3@sha256:…
CDC_PGCOPY_IMAGE=<ns>/cdc-pgcopydb:v1.2.3@sha256:…
```

The file is printed in the step log and attached to the Discord message. To deploy a
release, copy it to the production host and run
`docker/production/deploy.sh up --images images-<version>.env` (pull-only stack,
ordered rollout with health gates, rollback history; see `docker/production/README.md`).

## Images

| Image | Compose variable | Dockerfile | Build context / extra |
|-------|------------------|-----------|-----------------------|
| `cdc-orchestrator` | `CDC_ORCHESTRATOR_IMAGE` | `docker/Dockerfile` | repo root |
| `cdc-connect` | `CDC_CONNECT_IMAGE` | `docker/connect/Dockerfile` | repo root |
| `cdc-webui` | `CDC_NATIVE_UI_IMAGE` | `cdc-native-ui/Dockerfile` | `cdc-native-ui` |
| `cdc-mcp` | `MCP_CDC_IMAGE` | `mcp/Dockerfile` | `mcp` |
| `cdc-jdbc-admin` | `CDC_JDBC_ADMIN_IMAGE` | `docker/jdbc-admin/Dockerfile` | `docker/jdbc-admin` |
| `cdc-wireguard` | `CDC_WIREGUARD_IMAGE` | `docker/wireguard/Dockerfile` | `docker/wireguard` |
| `cdc-pgcopydb` | `CDC_PGCOPY_IMAGE` | `docker/pgcopydb/Dockerfile` | + `common=docker/sidecar_common`, `PG_MAJOR=18` |
| `cdc-pgdump` | `CDC_PGDUMP_IMAGE` | `docker/pgdump/Dockerfile` | `PG_MAJOR=18` |
| `cdc-pgloader` | `CDC_PGLOADER_IMAGE` | `docker/pgloader/Dockerfile` | + `common=docker/sidecar_common` |
| `cdc-redisshake` | `CDC_REDIS_SHAKE_IMAGE` | `docker/redisshake/Dockerfile` | + `common=docker/sidecar_common` |
| `cdc-mongocopy` | `CDC_MONGOCOPY_IMAGE` | `docker/mongocopy/Dockerfile` | + `common=docker/sidecar_common` |
| `cdc-rsync` | `CDC_RSYNC_IMAGE` | `docker/rsync/Dockerfile` | `docker/rsync` |

All are pushed as `$REGISTRY_NAMESPACE/<image>`. Published names `cdc-webui` and `cdc-mcp`
differ from the compose `:local` defaults (`cdc-native-ui`, `mcp-cdc`); the compose
variables are what tie them together.

The orchestrator image needs the repo root as its context because `docker/Dockerfile`
copies `Cargo.toml`, `Cargo.lock`, `build.rs`, `proto/`, and `src/`. The Connect image
also uses the repo root so it can compile the vendored `kafka-connect-jdbc/` fork.
The `common` named context mirrors `additional_contexts` in
`docker/docker-compose.sidecars.yaml`. Keep the two in sync.

`PG_MAJOR=18` matches the compose default. `pg_dump` must be at least the newest source
server's major version, so bump it here and in compose together.

## Required secrets

| Secret | Used by |
|--------|---------|
| `DOCKERHUB_USERNAME` | `build-and-push` |
| `DOCKERHUB_TOKEN` | `build-and-push` (needs read/write: it also pushes `:buildcache`) |
| `REGISTRY_NAMESPACE` | `build-and-push`, `notify-discord` (image name prefix) |
| `DISCORD_WEBHOOK` | `notify-discord` |
| `SONAR_TOKEN` | `sonar` (token for `https://sonarqube.b-quik-connect.com`, project `hyper-cdc-orchestrator`) |

All steps need the secrets above (no pull-request path).

## Notes

- Rust is pinned to `1.97-bookworm` here and in every Dockerfile, matching local
  development. `cargo fmt` and `clippy` results drift between rustfmt versions, so keep
  these in sync when bumping. Same for `golang:1.26` (wireguard), `maven:3.9.9-eclipse-temurin-17`
  (jdbc-admin, Connect) and `python:3.12` (sidecars).
- Shell expansions in `commands:` must be written `$${VAR}` so Woodpecker does not
  substitute them before the step runs. Inside an unquoted heredoc, backticks must be
  escaped (`` \` ``) or the shell runs them as command substitution.
- Multi-arch speed: the Go (wireguard) and Maven (jdbc-admin, Connect deps) build stages
  run on `$BUILDPLATFORM` and only the runtime stage is per-arch. The two Rust images
  still compile under arm64 emulation and dominate publish time; native arm64 agents
  are the next lever.
- `docker/redisshake` pins its upstream base by digest (upstream publishes only
  `latest`). Bump it deliberately; the command is in the Dockerfile comment.
