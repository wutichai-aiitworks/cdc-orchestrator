# secrets/

Docker secrets for the production stack: one file per credential, holding just the value
(no `KEY=`, no trailing newline). Only this README is committed. Everything else here is
gitignored and lives only on the production host.

Create them on the host (never on a laptop, never copied through chat or tickets):

```bash
cd docker/production
./deploy.sh secrets                            # generates the random ones, chmod 700 this dir
printf '%s' '<value>' > secrets/sso_app_secret
printf '%s' '<value>' > secrets/o2_password
./deploy.sh preflight
```

After `./deploy.sh secrets` the folder contains:

```
secrets/
├── orch_api_token            generated   orchestrator, native-ui, mcp-cdc
├── orch_secret_key           generated   orchestrator (32 B base64, keep stable)
├── orch_jdbc_admin_token     generated   orchestrator, jdbc-admin
├── orch_pgcopy_token         generated   orchestrator, pgcopydb
├── orch_redis_shake_token    generated   orchestrator, redisshake
├── orch_pgloader_token       generated   orchestrator, pgloader
├── orch_mongocopy_token      generated   orchestrator, mongocopy
├── orch_pgdump_token         generated   orchestrator, pgdump
├── orch_rsync_token          generated   orchestrator, rsync
├── orch_wireguard_token      generated   orchestrator, wireguard
├── admin_password            generated   native-ui (break-glass login)
├── session_secret            generated   native-ui
├── admin_session_secret      generated   native-ui
├── sso_app_secret            YOU FILL    native-ui
├── authelia_client_secret    optional    native-ui
├── mcp_client_keys           generated   mcp-cdc  (name:scope:token,…)
├── mcp_bearer_token          optional    mcp-cdc
├── o2_password               YOU FILL    cdc-vector
├── deploy_discord_webhook    optional    deploy.sh notifications
└── deploy_o2_auth_b64        optional    deploy.sh notifications
```

Rotate a generated secret: `./deploy.sh secrets --rotate <name> && ./deploy.sh up`.
See `../README.md` § Secrets for how they are mounted.
