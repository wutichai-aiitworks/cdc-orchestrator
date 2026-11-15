# ADR 0020 — "Connection method: SSH tunnel" for pipeline sources and targets

Status: Accepted (built; gateway and orchestrator unit-tested; see "Not verified")
Owner: Wutichai
Last updated: 2026-11-15

## Context

Some sources and targets expose **no database port**. The common case is a site published through
**Cloudflare Zero Trust** with only an SSH application (`cloudflared access ssh`). JDBC and Debezium
have no SSH option. `sslmode=require` encrypts the connection but does not make a closed port
reachable.

The WireGuard tunnels (ADR 0010) do not fit this case. They need a UDP port open at the site, which
Cloudflare does not carry. They are also wired only into the Copy & Sync tools, not into CDC pipelines.

There was no single place in the orchestrator to swap the address. A source/target connection's
host and port leave the orchestrator through five separate paths:

- Kafka Connect configs (Debezium, JDBC source/sink, trigger, incremental);
- `JdbcAdminClient`, which has no `AppState`;
- `connector_catalog::postgres_client`;
- direct MySQL / SQL Server clients in `trigger_cdc/exec.rs`;
- raw TCP probes, including the auto-recover port check.

The user asked for a choice on every pipeline source and target: **Direct** or **SSH tunnel**.

## Decision

1. **A `cdc-sshtunnel` gateway sidecar** (Go, `golang.org/x/crypto/ssh`), hardened the same way as
   `cdc-wireguard`: non-root, read-only filesystem, no capabilities.
   - It keeps one SSH session per tunnel.
   - Each forward is a TCP listener on `cdc-sshtunnel:<listen_port>` that dials
     `remote_host:remote_port` through the session (`ssh -L`).
   - **Cloudflare Access:** the SSH transport is the stdin/stdout of
     `cloudflared access ssh --hostname H`, the same mechanism as `ProxyCommand`. A service token is
     passed in the child's environment only, never on argv.
2. **Forwards are declared by the orchestrator, with a fixed port, and stored on the tunnel.** A
   Debezium connector stores host:port in its config, so the port must stay the same after a gateway
   restart. WireGuard forwards are first-free and in memory, so they cannot give that guarantee.
   - The orchestrator allocates `listen_port` once, from `22000–22999` (lowest free across all
     tunnels, under a mutex).
   - The full spec is pushed on every reconcile (every 30 s, when `version` differs).
   - Adding a forward never restarts the SSH session. The gateway compares a connection key that
     leaves out forwards and name.
3. **The connection is materialized, not rewritten at every egress.**
   `ssh_tunnel::materialize_conn` keeps the operator's intent in `ssh_tunnel_id`, `ssh_remote_host`
   and `ssh_remote_port`, and sets `host = cdc-sshtunnel`, `port = <listen_port>`. All five egress
   paths then work unchanged (every component is on `server_zone`).
   - It runs at the request edges: create/update profile, validate source/target, readiness,
     catalog, table inspect, trigger preview/install/status, target DDL.
   - It also runs in `reconcile_start`, so a forward exists before Connect gets the config.
   - Switching back to Direct restores the real host/port and drops the `ssh_*` keys.
   - Copy & Sync tools (`SchemaCompareConn.ssh_tunnel_id`, rsync endpoints) take the same route
     through `wireguard::route_payload`, at run/test time, the same way WireGuard works there.
4. **Listeners are open only while SSH is up.** While the session is down, callers get "connection
   refused" instead of a port that accepts and then drops. This keeps `probe_endpoint` and
   auto-recover accurate: Kafka Connect does not retry a FAILED task on its own, so auto-recover
   restarts the failed tasks when the port answers again. The Postgres slot keeps the position, so a
   tunnel blip loses no data.
5. **Host keys are pinned.**
   - A tunnel without `host_key_fingerprint` is stored but never pushed.
   - `POST /cdc/ssh-tunnels/test` stops before authentication when the key is unpinned or does not
     match, and returns the observed `SHA256:` fingerprint so the operator can confirm and pin it.
     No credential is ever sent to an unconfirmed server.
6. **Secrets.** The private key, passphrase, password and Cloudflare service-token secret are:
   - encrypted at rest with `secretbox` when `ORCH_SECRET_KEY` is set (ADR 0015);
   - redacted as `********` in the API;
   - restored from the stored tunnel only while the login (host, port, user, proxy) is unchanged
     (the same rule as ADR 0005 §11).
7. **A tunnel cannot be deleted while a pipeline uses it** (409, listing the pipeline names).
8. **API:** `/cdc/ssh-tunnels` (list, create), `/:id` (update, delete), `/test`, `/:id/probe`,
   `/:id/reconnect`, `/capability`. In the UI:
   - the `/ssh-tunnels` page;
   - the "Connection method" selector (`components/ConnectionMethod.vue`) in the wizard's source and
     target steps;
   - SSH tunnels in "Connect through" (`TransferTunnelSelect.vue`).
   The UI proxy is one catch-all route. Each extra typed Nitro route pushed `app.vue`'s `useFetch`
   into TS2589, so `app.vue` now uses `useAsyncData` + `useRequestFetch`.

## Consequences

- One more sidecar (about 30 MB including `cloudflared`), plus `ORCH_SSHTUNNEL_URL`,
  `ORCH_SSHTUNNEL_TOKEN`, and the `orch_sshtunnel_token` secret in production.
- A forward stays allocated after a pipeline stops using it. Ports are cheap (1000 in the range);
  pruning is a follow-up.
- `capture_identity` treats `ssh_remote_host@tunnel|ssh_remote_port|database` as the source identity,
  so two pipelines on the same database behind one tunnel are still detected as duplicates.
- All traffic for a tunnel is one SSH session, and through Cloudflare it is one long-lived stream.
  Throughput is bounded by that. A large `pg_dump -Fc` (one connection) is slow over it. For large
  initial loads, dump at the site and transfer the file, or use `pg_dump -Fd -j N` there.
- Forward allocation is serialised per orchestrator instance. Multi-instance deployments must not
  allocate on two instances at the same moment (in practice, one orchestrator per stack).
- `verify-full` TLS fails, because the hostname is `cdc-sshtunnel`. Use `sslmode=require` if the
  database requires TLS; SSH already encrypts the path.

## Not verified

- The gateway is tested against an in-process SSH server: round trip, adding a forward keeps the
  session, listener closed while down and the same port after reconnect, unpinned key returns the
  fingerprint without auth, mismatch refused, forwarding refused gives a hint. The race detector is
  clean.
- **Cloudflare Access end to end** (`cloudflared` subprocess + service token) has not been run
  against a real Access application. The image ships `cloudflared` 2025.4.0.

## Rejected

- **Rewrite at each egress** (WireGuard style): five paths, `JdbcAdminClient` has no state, and
  stored Connect configs would still point at unstable ports.
- **A static ops sidecar per site** (autossh in compose): works, but it is not selectable per
  pipeline, has no UI, and secrets end up in env files.
- **WireGuard over Cloudflare:** Cloudflare Tunnel does not carry UDP.
- **Cloudflare TCP route (`cloudflared access tcp`) instead of SSH:** needs the site admin to publish
  the database port as an Access TCP application. Possible later as a third connection method, but
  it is not what these sites offer today.
