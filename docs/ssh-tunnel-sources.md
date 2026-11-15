# Sources and targets behind SSH / Cloudflare Zero Trust

Use **Connection method → SSH tunnel** when a database port is not reachable but SSH is. The usual
case is a site that publishes only a **Cloudflare Access SSH application**. Design and decisions:
[ADR 0020](adr/0020-ssh-tunnel-connection-method.md). To plan the first copy of a large Postgres
source, see [postgres-large-source-cdc.md](postgres-large-source-cdc.md).

```
cdc-connect / jdbc-admin / orchestrator ─► cdc-sshtunnel:22xxx ─ssh -L─► [cloudflared access ssh] ─► SSH server ─► db:5432
```

Contents:

- [How it works](#how-it-works)
- [Site preparation](#site-preparation)
- [Create a tunnel](#create-a-tunnel)
- [Host key pinning](#host-key)
- [Cloudflare Access vs Direct SSH](#cloudflare-access)
- [Use it in a pipeline](#use-in-a-pipeline)
- [Copy & Sync tools](#copy-and-sync)
- [Operations](#operations)
- [Troubleshooting](#troubleshooting)
- [Configuration](#configuration)
- [API samples](#api-samples)

<a id="how-it-works"></a>
## How it works

- The **`cdc-sshtunnel`** gateway keeps one SSH session per tunnel. For each database it listens on
  a fixed port (`cdc-sshtunnel:22000`–`22999`) and forwards connections through the session (the
  same thing `ssh -L` does).
- In the pipeline you enter the database **as seen from the SSH server** (often `127.0.0.1:5432`).
  When the pipeline is saved or tested, the orchestrator replaces host/port with
  `cdc-sshtunnel:<port>`. Kafka Connect, jdbc-admin and the tools all use that address.
- A forward port **never changes**, including across gateway restarts. Debezium keeps host:port in
  its config, so it must stay stable.
- While SSH is down, the forward ports are closed. A failed Debezium task is restarted
  automatically once the port answers again. The replication slot keeps the position, so no
  changes are lost.

<a id="site-preparation"></a>
## Site preparation

| Item | What to do |
|------|-----------|
| Cloudflare Access | Create a **service token** (Zero Trust → Access → Service Auth). On the SSH application, add a policy with action **Service Auth** that includes this token. Send us the Client ID and Client Secret. |
| SSH server | `AllowTcpForwarding yes` in `sshd_config` (the default on most distros). |
| SSH user | Create a dedicated user with key-only login. Restrict its key to the database:<br>`restrict,port-forwarding,permitopen="127.0.0.1:5432" ssh-ed25519 AAAA… hyper-cdc`<br>Use the database host/port as seen from this server, and repeat `permitopen` for each database. |
| Host key | Send the fingerprint: `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub`. |
| PostgreSQL `pg_hba.conf` | Allow the SSH server's address (`127.0.0.1/32` when SSH runs on the DB host). Add **both** lines, because Debezium opens a *replication* connection:<br>`host  appdb        cdc_user  127.0.0.1/32  scram-sha-256`<br>`host  replication  cdc_user  127.0.0.1/32  scram-sha-256` |
| MySQL / SQL Server / Oracle | Grant the CDC login from the SSH server's address. |

Create a key pair for the tunnel on your side (never reuse a personal key):

```bash
ssh-keygen -t ed25519 -N "" -C hyper-cdc -f cdc_tunnel_ed25519
# send cdc_tunnel_ed25519.pub to the site; paste cdc_tunnel_ed25519 into the tunnel form
```

<a id="create-a-tunnel"></a>
## Create a tunnel

UI → **Tunnels → SSH** → **New SSH tunnel**:

1. Choose **Cloudflare Access** or **Direct SSH**, then fill in the host, user and private key (and
   the service token for Cloudflare).
2. Click **Test SSH**. The first test stops at the host key; see [Host key pinning](#host-key).
3. Click **Test SSH + database** with the database address as seen from the SSH server.
4. Save. Within a few seconds the tunnel shows **Connected**.

<a id="host-key"></a>
## Host key pinning

- A tunnel runs only with a pinned host key (`SHA256:…`).
- **Test SSH** without a pin stops *before* logging in and shows the server's fingerprint, so no
  key or password is sent to an unconfirmed server.
- Compare the fingerprint with what the site sent (`ssh-keygen -lf
  /etc/ssh/ssh_host_ed25519_key.pub`), then click **Pin**. The gateway prefers ed25519 host keys,
  as OpenSSH does.
- *Host key mismatch* means the server's key changed or this is a different server. Confirm with
  the site before you update the pin.

<a id="cloudflare-access"></a>
## Cloudflare Access vs Direct SSH

Both options end with the same SSH session and the same forwards. They differ only in **how the
gateway reaches the SSH server**.

| | **Cloudflare Access** | **Direct SSH** |
|---|---|---|
| Use when | The site publishes SSH only through Cloudflare Zero Trust; no port is open to us | We can reach the SSH server over the network: public IP with port 22 allowed, VPN, LAN, WireGuard |
| Path | gateway → `cloudflared access ssh` → Cloudflare edge → site's `cloudflared` tunnel → sshd | gateway → TCP → sshd |
| Site opens a port | **No** (the site's cloudflared dials out to Cloudflare) | Yes, SSH (22 or custom) for our source IP |
| Form fields | **SSH hostname** = the Access application hostname (e.g. `ssh.branch.example.com`), **Service token ID + secret** | **SSH host** + **SSH port** |
| Authentication | Two layers: Cloudflare checks the service token, then sshd checks the key | sshd checks the key |
| Site prepares | Access SSH application + **Service Auth** policy for our token, plus the SSH user/key | Firewall rule for our IP, plus the SSH user/key |
| Latency / throughput | Extra hop through the Cloudflare edge; fine for CDC, slower for bulk copies | Direct; best for large initial loads |
| Typical errors | Session closes right after start → token or policy problem | `connection refused` / timeout → firewall or port |

The same in both cases: the SSH user and key, `AllowTcpForwarding`, `permitopen`, host key
pinning, the database host as seen from the SSH server, and `pg_hba.conf`.

**What Cloudflare Access does:** on the gateway this is equivalent to

```bash
ssh -o ProxyCommand="cloudflared access ssh --hostname ssh.branch.example.com" \
    -L 22000:127.0.0.1:5432 cdc-tunnel@ssh.branch.example.com
# with TUNNEL_SERVICE_TOKEN_ID / TUNNEL_SERVICE_TOKEN_SECRET in cloudflared's environment
```

A server cannot do the browser login that people use, so the service token is required. In Zero
Trust, the SSH application's policy must have action **Service Auth** and include this token. An
*Allow* policy for users does not apply to tokens.

**What Direct SSH does:**

```bash
ssh -p 22 -L 22000:127.0.0.1:5432 cdc-tunnel@bastion.site.example
```

To try the same path from your laptop before creating the tunnel, run one of the commands above
and connect `psql -h 127.0.0.1 -p 22000`.

<a id="use-in-a-pipeline"></a>
## Use it in a pipeline

In the wizard's **Source** (or **Target**) step:

1. Set **Connection method** to **SSH tunnel** and pick the tunnel.
2. Set **Host / Port** to the database **as seen from the SSH server** (often `127.0.0.1` / `5432`).
3. Click **Test path to database**, then **Test connection** and **Check source** as usual.

After saving, the selector shows the pipeline's fixed address (for example
`cdc-sshtunnel:22000`). Switching back to **Direct** restores the host/port you entered.

<a id="copy-and-sync"></a>
## Copy & Sync tools

pg_dump, pgcopydb, pgloader, Mongo copy, Redis sync and File sync list SSH tunnels under **Connect
through**. Host/port are again the address as seen from the SSH server. File sync through a tunnel
always uses the relay transport.

<a id="operations"></a>
## Operations

- **Tunnel drops:** the forward ports close, and auto-recover restarts the failed tasks once SSH is
  back. **Retry now** on the **Tunnels → SSH** page skips the reconnect backoff (up to 30 s).
- **Delete:** refused while a pipeline uses the tunnel. Switch those pipelines to Direct first.
- **Throughput:** all traffic for a tunnel goes through one SSH stream (through Cloudflare, one
  long-lived connection). For a large first copy, see
  [postgres-large-source-cdc.md](postgres-large-source-cdc.md#large-dump-over-ssh).
- **TLS:** SSH already encrypts the path. If the database requires TLS, use `sslmode=require`.
  `verify-full` fails because the hostname is `cdc-sshtunnel`.

<a id="troubleshooting"></a>
## Troubleshooting

| Message | Cause / fix |
|---------|-------------|
| `SSH tunnels are not configured` | `ORCH_SSHTUNNEL_URL` is empty, or `cdc-sshtunnel` is not running. See [Configuration](#configuration). |
| `cdc-sshtunnel unreachable` | The container is down or not on the `server_zone` network. |
| `rejected the token` | `ORCH_SSHTUNNEL_TOKEN` ≠ the gateway's `SSHTUNNEL_API_TOKEN`. |
| `host key not pinned` | Expected on the first test. Compare and pin; see [Host key pinning](#host-key). |
| `host key mismatch` | The server key changed. Confirm with the site before re-pinning. |
| `unable to authenticate` | Wrong user, or the public key is not in `authorized_keys`. |
| `administratively prohibited` | `AllowTcpForwarding no`, or the host:port is not in `permitopen`. |
| `connect refused` (through SSH) | SSH works, but nothing listens at that address *as seen from the SSH server*. |
| Cloudflare: session closes right away | The service token has no **Service Auth** policy on the SSH app, or the Access hostname is wrong. |
| `the private key is encrypted` | Enter the key passphrase. |
| `no free SSH tunnel forward port` | The 1000-port range is used up; widen `SSHT_FORWARD_PORT_MIN/MAX`. |

<a id="configuration"></a>
## Configuration

| Variable | Where | Default / meaning |
|----------|-------|-------------------|
| `ORCH_SSHTUNNEL_URL` | orchestrator | `http://cdc-sshtunnel:8092`: the gateway's **control API** (tunnel push, status, tests). Empty = feature off. It is *not* the address pipelines connect to. |
| `ORCH_SSHTUNNEL_TOKEN` / `SSHTUNNEL_API_TOKEN` | orchestrator / gateway | Bearer token for the control API; the same value on both sides. Production secret `orch_sshtunnel_token`. |
| `SSHT_FORWARD_PORT_MIN` / `MAX` | both (compose passes them to the orchestrator as `ORCH_SSHTUNNEL_PORT_*`) | `22000` / `22999`: forward ports (the data path). |
| `ORCH_SSHTUNNEL_FORWARD_HOST` | orchestrator | `cdc-sshtunnel`: the host written into pipeline connections. |

Port 8092 is not published on the host; only containers on `server_zone` reach it.

**Local / dev**: `docker/.env`

```dotenv
ORCH_SSHTUNNEL_URL=http://cdc-sshtunnel:8092
ORCH_SSHTUNNEL_TOKEN=<openssl rand -hex 32>
SSHT_FORWARD_PORT_MIN=22000
SSHT_FORWARD_PORT_MAX=22999
```

Then `./docker/deploy.sh` (the default redeploy starts `cdc-sshtunnel` too).

**Production**: `docker/production/`

```bash
openssl rand -hex 32 > secrets/orch_sshtunnel_token   # deploy.sh generates it if missing
chmod 600 secrets/orch_sshtunnel_token
./deploy.sh
```

**Check**

```bash
TOKEN=$(docker exec cdc-orchestrator printenv ORCH_API_TOKEN)
curl -s -H "Authorization: Bearer $TOKEN" localhost:9788/cdc/ssh-tunnels/capability
# {"configured":true,"reachable":true,"engine":"ssh","forward_ports":"22000-22999","cloudflared":"cloudflared version …"}
```

| Response | Meaning |
|---|---|
| `{"configured":false}` | `ORCH_SSHTUNNEL_URL` is empty |
| `reachable:false` + `unreachable` | The gateway is down or not on `server_zone` |
| `reachable:false` + `rejected the token` | The tokens differ |

<a id="api-samples"></a>
## API samples

Create a tunnel (Cloudflare Access):

```bash
curl -s -X POST localhost:9788/cdc/ssh-tunnels \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d @- <<EOF
{
  "name": "branch-db (Cloudflare)",
  "host": "ssh.branch.example.com",
  "username": "cdc-tunnel",
  "auth_method": "key",
  "private_key": $(jq -Rs . < cdc_tunnel_ed25519),
  "host_key_fingerprint": "SHA256:<from Test SSH>",
  "proxy": {
    "kind": "cloudflare_access",
    "hostname": "ssh.branch.example.com",
    "service_token_id": "<client-id>.access",
    "service_token_secret": "<client-secret>"
  }
}
EOF
```

Test a form before saving (it returns the fingerprint when none is pinned):

```bash
curl -s -X POST localhost:9788/cdc/ssh-tunnels/test -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"tunnel": { …same fields… }, "host": "127.0.0.1", "port": 5432}'
```

Pipeline source connection through the tunnel:

```json
"connection": {
  "host": "127.0.0.1", "port": "5432",
  "database": "appdb", "username": "cdc_user", "password": "…",
  "ssh_tunnel_id": "<tunnel id>",
  "ssh_remote_host": "127.0.0.1", "ssh_remote_port": "5432"
}
```

After saving, `host`/`port` read `cdc-sshtunnel` / `22000` (for example). The `ssh_remote_*` keys
keep the address you entered.

| Endpoint | Purpose |
|---|---|
| `GET /cdc/ssh-tunnels` | List tunnels with status, forwards and `used_by` |
| `POST /cdc/ssh-tunnels` · `PUT /cdc/ssh-tunnels/{id}` · `DELETE …/{id}` | Create / update / delete (409 while used) |
| `POST /cdc/ssh-tunnels/test` | SSH login (+ optional DB dial) for a form |
| `POST /cdc/ssh-tunnels/{id}/probe` | Dial host:port through the running session |
| `POST /cdc/ssh-tunnels/{id}/reconnect` | Skip the reconnect backoff |
| `GET /cdc/ssh-tunnels/capability` | Gateway configured / reachable |
