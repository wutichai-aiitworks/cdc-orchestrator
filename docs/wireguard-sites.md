# Connecting to a site over WireGuard

For source and target databases that are **not reachable from outside** their network. The site
opens **one UDP port** for WireGuard; its database ports stay closed. Hyper CDC's `cdc-wireguard`
gateway dials that port, and the Copy & Sync tools reach the database through the tunnel.

```
 Hyper CDC server                                   Source site (or target site)
┌──────────────────────────────┐   WireGuard UDP   ┌───────────────────────────────────────┐
│ pgcopy / redisshake / …      │      51820        │ WireGuard server   10.66.1.1          │
│        │ TCP                 │ ════════════════▶ │        │ (forward + NAT, if the DB    │
│ cdc-wireguard  10.66.1.2 ────┼───────────────────┼────────┘  is on another host)        │
└──────────────────────────────┘                   │ Database  192.168.10.5:5432  (closed) │
                                                   └───────────────────────────────────────┘
```

Each site is one tunnel. A source and a target behind different firewalls are two tunnels; each
connection in a profile picks its own under **Connect through**.

## 0. Prepare the site first

**WireGuard tunnels → Prepare a site** (also shown at the top of **New tunnel**) lists what to
collect from the site and the steps for its platform (OS, host firewall, router or cloud security
group), and produces a checklist to send them (**Copy checklist** / **Download .md**). In short:

1. Collect: the site's public address, the UDP port (default 51820), where the database runs and its
   host:port as seen from the site, the networks already used there (to avoid clashing with 10.66.x.x).
2. Install WireGuard on the chosen host; allow the UDP port in its firewall and at the edge (router
   port-forward or cloud security group), ideally only from Hyper CDC's public IP.
3. **Test the port before any tunnel exists**: the site runs `sudo socat -v UDP4-LISTEN:51820,fork PIPE`
   and you press **Test UDP port**. The gateway sends a test datagram:
   - *UDP port open: the echo came back* — firewall, port-forward and NAT are right.
   - *Nothing listens on this UDP port* — packets reach the host but the echo is not running (or a
     different port).
   - *No echo came back* — still blocked (host firewall, security group, router) or wrong address.
   Stop the echo (Ctrl+C) before starting WireGuard on that port.

## 1. Create the tunnel (Hyper CDC)

**Copy & Sync → WireGuard tunnels → New tunnel.** You need from the site:

| Field | What to enter |
|---|---|
| Site WireGuard endpoint | The site's public address and the UDP port it will open, e.g. `vpn.source.example:51820` |
| Site's public key | Either paste the site's `wg pubkey`, or leave **Generate the site's keys here** on (the site's private key is then shown once, inside the configuration) |
| Reachable through the tunnel | The site's tunnel address (`10.66.1.1/32`, prefilled) **plus** the database host or network when the database is on another machine, e.g. `192.168.10.0/24` |

Tunnel addresses are suggested per tunnel (`10.66.N.2` here, `10.66.N.1` at the site). Change them
only if `10.66.0.0/16` is already used at the site.

Saving shows **Site setup**: the complete `wg0.conf` for the site and a checklist. Copy or download
it then — the site's private key and the preshared key are shown only once and not stored here.
**Site setup** on an existing tunnel shows the configuration again, with placeholders for those keys.

## 2. What the site configures

Everything the site needs is in the generated configuration. In short:

### a) One inbound UDP port

Open the WireGuard port (default `51820/udp`) from the internet to the WireGuard server, on the
router/firewall (port forward if it is behind NAT). **Nothing else is opened**; database ports
stay closed. If you can, restrict the source address to the Hyper CDC server's public IP.

### b) A WireGuard server

On a Linux host at the site (the database server itself, or any host that can reach it):

```bash
sudo apt install wireguard          # Debian/Ubuntu   (RHEL: sudo dnf install wireguard-tools)
sudo install -m 600 /dev/stdin /etc/wireguard/wg0.conf < wg0.conf   # the file from Site setup
sudo systemctl enable --now wg-quick@wg0
```

The generated file looks like this (values differ):

```ini
[Interface]
Address    = 10.66.1.1/32
ListenPort = 51820
PrivateKey = <the site's private key>
# Only when the database is on another host:
PostUp   = iptables -A FORWARD -i %i -s 10.66.1.2/32 -j ACCEPT; iptables -A FORWARD -o %i -m state --state RELATED,ESTABLISHED -j ACCEPT; iptables -t nat -A POSTROUTING -s 10.66.1.2/32 -o eth0 -j MASQUERADE
PostDown = iptables -D FORWARD -i %i -s 10.66.1.2/32 -j ACCEPT; iptables -D FORWARD -o %i -m state --state RELATED,ESTABLISHED -j ACCEPT; iptables -t nat -D POSTROUTING -s 10.66.1.2/32 -o eth0 -j MASQUERADE

[Peer]
# Hyper CDC gateway — it dials in, so no Endpoint here
PublicKey    = <the gateway's public key>
PresharedKey = <shown once>
AllowedIPs   = 10.66.1.2/32
```

**Database on another host** (the file then has `PostUp`/`PostDown`):

- Set `eth0` in `PostUp`/`PostDown` to the interface that reaches the database network.
- Turn on IP forwarding persistently:
  `echo 'net.ipv4.ip_forward=1' | sudo tee /etc/sysctl.d/99-wireguard.conf && sudo sysctl --system`
- With masquerading, the database sees connections from the WireGuard host's LAN address, not
  `10.66.1.2` — allow that address in the database (below).

**Appliances** (OPNsense, pfSense, MikroTik, UniFi, Fortinet…) work the same: create a WireGuard
interface with the site address and port above, add one peer with the gateway's public key,
preshared key and `AllowedIPs = 10.66.1.2/32`, no endpoint, and allow it to reach the database.

### c) Let the database accept the tunnel

Allow the gateway's tunnel address (`10.66.1.2`), or the WireGuard host's LAN address when it
masquerades:

| Engine | Listen on | Allow |
|---|---|---|
| PostgreSQL | `listen_addresses` covers the address you connect to (`'*'` or the LAN/tunnel IP) | `pg_hba.conf`: `host all <user> 10.66.1.2/32 scram-sha-256` (replication too for Postgres copy with live changes: `host replication <user> 10.66.1.2/32 scram-sha-256`) |
| Redis | `bind` includes that address; keep `protected-mode yes` with a password | `requirepass` or an ACL user (Redis sync needs `SYNC`/`PSYNC`; *Empty target first* needs `FLUSHDB`/`FLUSHALL`) |
| MongoDB | `net.bindIp` includes that address | a user with the needed roles; connect to one member directly (replica-set discovery hands out member names the tunnel does not forward) |
| MySQL / MariaDB | `bind-address` | `CREATE USER 'cdc'@'10.66.1.2' …` |
| SQL Server | TCP enabled on the port | login for the tool |
| SSH (File sync) | `sshd` listening | the account's key or password |

Host firewalls on the database host (ufw, firewalld, security groups) must allow the same source.

### d) Check

On the site: `sudo wg show` lists the peer with a recent **latest handshake** shortly after the
tunnel is saved in Hyper CDC (the gateway keeps it alive every 25 s).

In Hyper CDC, each tunnel has a **Readiness** panel (WireGuard tunnels page) that walks through
every step and checks it live: gateway running → tunnel loaded → site endpoint resolves →
handshake (UDP port, WireGuard, gateway key) → forwarding/NAT (only when the database is on another
host) → database listens and allows the tunnel → login (via the profile's Test connection). Enter
the database's host and port as seen from the site and press **Check through tunnel**; each step
shows what to fix and the commands for the site.

### e) File sync with an rsync daemon (optional)

Instead of SSH logins, File sync can use the site's `rsync --daemon` (`rsync://`, TCP 873) — see
ADR 0012. The protocol is not encrypted, so serve it **on the tunnel address only**:

```ini
# /etc/rsyncd.conf
address = 10.66.1.1            # the site's tunnel address
port = 873
use chroot = yes

[data]                          # module name used in File sync
path = /srv/data
read only = no                  # "yes" when the site is only a source
uid = root                      # keeps owners (or a user that owns /srv/data)
gid = root
hosts allow = 10.66.1.2         # the gateway's tunnel address
hosts deny = *
auth users = cdc
secrets file = /etc/rsyncd.secrets   # "cdc:password", chmod 600
```

`sudo systemctl enable --now rsync`, then in File sync: **Connect with → rsync daemon**, host
`10.66.1.1`, port 873, this tunnel, module `data`, user/password → **Test connection**. Tunnel
Readiness → *rsync daemon (File sync)* shows the same lines for your addresses.

## 3. Use it in a profile

On the source or target connection: **Connect through → WireGuard: <tunnel>**, then enter the host
and port **as seen from the site** — its LAN address (`192.168.10.5`) or, when the database runs on
the WireGuard server, the site's tunnel address (`10.66.1.1`). **Test connection** goes through the
tunnel too. Nothing else changes; runs, timeline and logs work as for direct connections.

| Tool | Through a tunnel |
|---|---|
| Postgres copy / dump, Load to Postgres, Mongo copy, Redis sync | Source and target connections |
| File sync | Source and target hosts; runs always use the relay (files pass through this server) |
| Not supported | Redis **Cluster** and MongoDB replica-set discovery (they redirect to node addresses), Postgres dump's SFTP destinations |

## Troubleshooting

| Symptom | Check |
|---|---|
| **No handshake yet** | UDP port open and forwarded to the WireGuard host; endpoint host/port right; the site's `[Peer] PublicKey` is the gateway's key from Site setup; the tunnel's *site public key* matches `wg pubkey` on the site |
| Handshake OK, Probe: *connection refused* | The database does not listen on that address/port (bind / `listen_addresses`) |
| Handshake OK, Probe: *timed out* | Database on another host: forwarding/NAT (`PostUp`, `ip_forward`, interface name); the database host's firewall; the address is inside *Reachable through the tunnel* |
| Login fails, Probe OK | The database's allow-list (`pg_hba.conf`, Redis ACL, MySQL user host) for `10.66.1.2` or the masquerading host's address |
| Host name does not resolve | Use an IP, or set the tunnel's **DNS at the site** to a resolver there |
