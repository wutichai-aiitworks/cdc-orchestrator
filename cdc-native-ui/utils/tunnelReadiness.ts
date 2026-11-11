// WireGuard tunnel readiness (ADR 0010): what must be true, on our side and at the site, before
// tools can reach a database through the tunnel — each step checked live where we can see it
// (gateway, handshake, a TCP probe through the tunnel), with the site's commands for the rest.
// Unit-tested in tests/unit/tunnelReadiness.test.ts.

export type ReadyState = "ok" | "fail" | "pending" | "skip" | "manual"
export type DbEngine = "postgres" | "redis" | "mongodb" | "mysql" | "ssh" | "rsyncd"

export const ENGINES: { value: DbEngine; label: string; port: number }[] = [
  { value: "postgres", label: "PostgreSQL", port: 5432 },
  { value: "redis", label: "Redis", port: 6379 },
  { value: "mongodb", label: "MongoDB", port: 27017 },
  { value: "mysql", label: "MySQL", port: 3306 },
  { value: "ssh", label: "SSH (File sync)", port: 22 },
  { value: "rsyncd", label: "rsync daemon (File sync)", port: 873 },
]

export type ReadinessStep = {
  key: string
  title: string
  who: "Hyper CDC" | "Site"
  state: ReadyState
  detail: string
  /** What to do when the step is not done. */
  fix?: string
  /** Commands for the site (copyable). */
  commands?: string
}

type TunnelLike = {
  address: string
  site_address?: string
  peer_endpoint: string
  allowed_ips: string[]
  /** Gateway's public key: the site's [Peer] PublicKey. */
  public_key?: string
  /** The site's public key the gateway expects (from the site's private key). */
  peer_public_key?: string
  status?: { connected: boolean; handshake_age_secs?: number | null; resolved_endpoint?: string } | null
}

export type ProbeLike = { ok: boolean; error?: string; target: string; ms?: number } | null

const ip = (cidr: string) => cidr.split("/")[0]
const portOf = (endpoint: string) => Number(endpoint.split(":").pop()) || 51820

/** Networks beyond the site's own tunnel address: the database is on another host (needs forwarding + NAT). */
export const lanNetworks = (t: TunnelLike) => t.allowed_ips.filter((a) => ip(a) !== ip(t.site_address || ""))

/** The address the database sees connections from: the gateway's tunnel IP, or the WireGuard host's LAN IP when it masquerades. */
export function dbSourceAddress(t: TunnelLike): { address: string; masquerade: boolean } {
  return lanNetworks(t).length ? { address: "<the WireGuard host's LAN address>", masquerade: true } : { address: ip(t.address), masquerade: false }
}

/** Commands that let the database accept connections from the tunnel. */
export function dbAllowCommands(engine: DbEngine, t: TunnelLike): string {
  const { address, masquerade } = dbSourceAddress(t)
  const a = masquerade ? "<wg-host-lan-ip>" : address
  const note = masquerade ? `# The WireGuard host masquerades: allow its LAN address (shown as ${a}).\n` : `# Allow the gateway's tunnel address ${a}.\n`
  switch (engine) {
    case "postgres":
      return `${note}# postgresql.conf — listen on the address the gateway connects to:
listen_addresses = '*'            # or the LAN / tunnel IP
# pg_hba.conf:
host  all          cdc_user  ${a}/32  scram-sha-256
host  replication  cdc_user  ${a}/32  scram-sha-256   # Postgres copy with live changes
sudo systemctl reload postgresql`
    case "redis":
      return `${note}# redis.conf:
bind 127.0.0.1 ${masquerade ? "<redis-lan-ip>" : ip(t.site_address || "10.66.1.1/32")}
protected-mode yes
# a login for the sync (see Redis sync permissions):
redis-cli ACL SETUSER cdc_writer on '>Strong-Password' '~*' -@all +@write +@read +@connection +select +info +restore
redis-cli ACL SAVE    # or CONFIG REWRITE`
    case "mongodb":
      return `${note}# mongod.conf:
net:
  bindIp: 127.0.0.1,${masquerade ? "<mongo-lan-ip>" : ip(t.site_address || "10.66.1.1/32")}
# connect to one member directly (replica-set discovery does not follow the tunnel)
sudo systemctl restart mongod`
    case "mysql":
      return `${note}# my.cnf:
bind-address = 0.0.0.0           # or the LAN / tunnel IP
mysql -e "CREATE USER 'cdc'@'${a}' IDENTIFIED BY 'Strong-Password'; GRANT SELECT, RELOAD, SHOW DATABASES, REPLICATION SLAVE, REPLICATION CLIENT ON *.* TO 'cdc'@'${a}';"`
    case "rsyncd": {
      const listen = masquerade ? "<rsync-host-lan-ip>" : ip(t.site_address || "10.66.1.1/32")
      return `${note}# rsync daemon (rsync://, port 873). Not encrypted: serve it on the tunnel address only.
sudo apt-get install -y rsync          # dnf install rsync / apk add rsync
sudo tee /etc/rsyncd.conf >/dev/null <<'CONF'
address = ${listen}                  # listen on the tunnel address only
port = 873
use chroot = yes
max connections = 4
log file = /var/log/rsyncd.log

[data]                                 # the module name used in File sync
path = /srv/data
read only = no                         # "yes" when this site is only a source
uid = root                             # keeps owners; or a dedicated user that owns /srv/data
gid = root
hosts allow = ${a}
hosts deny = *
auth users = cdc
secrets file = /etc/rsyncd.secrets
CONF
echo 'cdc:Strong-Password' | sudo tee /etc/rsyncd.secrets >/dev/null
sudo chmod 600 /etc/rsyncd.secrets     # rsync refuses a secrets file others can read
sudo systemctl enable --now rsync      # (rsyncd on RHEL/Alpine)
sudo ss -ltnp | grep ':873'`
    }
    case "ssh":
      return `${note}# sshd listens on all addresses by default; check:
sudo ss -ltnp | grep ':22'
# firewall on the host, if any:
sudo ufw allow from ${a} to any port 22 proto tcp`
  }
}

/**
 * Readiness steps for a tunnel. `probe` is the last "is host:port reachable through the tunnel"
 * check (null = not run yet).
 */
export function tunnelReadiness(t: TunnelLike, gatewayReachable: boolean, probe: ProbeLike, engine: DbEngine = "postgres"): ReadinessStep[] {
  const st = t.status
  const port = portOf(t.peer_endpoint)
  const gw = ip(t.address)
  const lan = lanNetworks(t)
  const steps: ReadinessStep[] = []

  steps.push({
    key: "gateway",
    title: "Hyper CDC gateway running",
    who: "Hyper CDC",
    state: gatewayReachable ? "ok" : "fail",
    detail: gatewayReachable ? "cdc-wireguard answers." : "The cdc-wireguard gateway does not answer.",
    fix: gatewayReachable ? undefined : "Start the cdc-wireguard container and set ORCH_WIREGUARD_URL on the orchestrator.",
  })

  steps.push({
    key: "loaded",
    title: "Tunnel loaded on the gateway",
    who: "Hyper CDC",
    state: !gatewayReachable ? "pending" : st ? "ok" : "fail",
    detail: st ? "The gateway has this tunnel." : "The gateway does not have this tunnel yet.",
    fix: st || !gatewayReachable ? undefined : "Save the tunnel again; the orchestrator also pushes it within 30 s. A bad key or endpoint is reported when saving.",
  })

  const resolved = !!st?.resolved_endpoint
  steps.push({
    key: "dns",
    title: "Site endpoint resolves",
    who: "Site",
    state: !st ? "pending" : resolved ? "ok" : "fail",
    detail: resolved ? `${t.peer_endpoint} → ${st!.resolved_endpoint}` : `${t.peer_endpoint} does not resolve from the gateway.`,
    fix: resolved || !st ? undefined : "Use the site's public IP, or fix the DNS name (A record) for its WireGuard endpoint.",
  })

  const handshake = !!st && st.handshake_age_secs != null
  const connected = !!st?.connected
  steps.push({
    key: "handshake",
    title: `Site WireGuard up, UDP ${port} open, gateway key added`,
    who: "Site",
    state: !st ? "pending" : connected ? "ok" : "fail",
    detail: connected
      ? `Handshake ${st!.handshake_age_secs} s ago.`
      : handshake
        ? `Last handshake ${st!.handshake_age_secs} s ago — the tunnel dropped.`
        : "No handshake yet. WireGuard stays silent on any mistake, so check all of these at the site.",
    fix: connected
      ? undefined
      : `Open UDP ${port} inbound (port-forward to the WireGuard host if behind NAT), start WireGuard with the generated wg0.conf, and make sure its [Peer] PublicKey is the gateway key from Site setup.` +
        (t.public_key && t.peer_public_key
          ? ` On the site, \`wg show wg0\` must list "public key: ${t.peer_public_key}" and "peer: ${t.public_key}" — other keys mean the site runs an older wg0.conf (e.g. from a deleted or re-keyed tunnel): install the current one.`
          : ""),
    commands: `${
      t.public_key && t.peer_public_key
        ? `# keys must match this tunnel
sudo wg show wg0 public-key   # expect ${t.peer_public_key}
sudo wg show wg0 peers        # expect ${t.public_key}
`
        : ""
    }# firewall (pick yours)
sudo ufw allow ${port}/udp
sudo firewall-cmd --permanent --add-port=${port}/udp && sudo firewall-cmd --reload
# WireGuard with the generated config
sudo install -m 600 wg0.conf /etc/wireguard/wg0.conf
sudo systemctl enable --now wg-quick@wg0
# should show "latest handshake" for the gateway peer within ~25 s
sudo wg show wg0`,
  })

  const probed = !!probe
  const refused = probed && !probe!.ok && /refused/i.test(probe!.error || "")
  const timedOut = probed && !probe!.ok && !refused

  steps.push({
    key: "forward",
    title: "Forwarding and NAT to the database host",
    who: "Site",
    state: !lan.length ? "skip" : !connected ? "pending" : !probed ? "manual" : timedOut ? "fail" : "ok",
    detail: !lan.length
      ? "Not needed: the database runs on the WireGuard host (only the site's tunnel address is reachable)."
      : !probed
        ? `Needed for ${lan.join(", ")}. Run the check below with the database's LAN address.`
        : timedOut
          ? `${probe!.target} did not answer through the tunnel.`
          : `${probe!.target} answered through the tunnel.`,
    fix: lan.length && timedOut ? "Turn on IP forwarding persistently and check the PostUp NAT rule's interface name (eth0 → the interface that reaches the database)." : undefined,
    commands: lan.length
      ? `echo 'net.ipv4.ip_forward=1' | sudo tee /etc/sysctl.d/99-wireguard.conf && sudo sysctl --system
# in wg0.conf PostUp/PostDown, replace eth0 with the interface toward the database:
ip -o -4 route get <database-lan-ip>
sudo systemctl restart wg-quick@wg0`
      : undefined,
  })

  steps.push({
    key: "listen",
    title: "Database listens and allows the tunnel",
    who: "Site",
    state: !connected ? "pending" : !probed ? "manual" : probe!.ok ? "ok" : "fail",
    detail: !probed
      ? "Run the check below with the database's host and port as seen from the site."
      : probe!.ok
        ? `${probe!.target} accepts connections (${probe!.ms} ms).`
        : refused
          ? `${probe!.target} refused the connection: nothing listens there.`
          : `${probe!.target} did not answer (firewall on the database host, or forwarding).`,
    fix: probed && !probe!.ok ? "Make the database listen on that address and allow the tunnel in its configuration and host firewall." : undefined,
    commands: dbAllowCommands(engine, t),
  })

  // The last step needs credentials, so it is confirmed where they are entered: in the tool itself.
  const host = ip(t.site_address || "10.66.1.1/32")
  steps.push(
    engine === "rsyncd"
      ? {
          key: "login",
          title: "rsync daemon module and login",
          who: "Site",
          state: "manual",
          detail: `A TCP check cannot verify a module or its password. In File sync, choose “rsync daemon” under “Connect with”, set the host to ${host} (port 873), choose this tunnel, enter the module (and its user/password), and press Test connection. A target module needs read only = no.`,
        }
      : engine === "ssh"
      ? {
          key: "login",
          title: "SSH login and folder access",
          who: "Site",
          state: "manual",
          detail: `A TCP check cannot verify a login. In File sync, set the host to ${host} (port 22), choose this tunnel under “Connect through”, enter the user and key, and press Test login. The user must read the source folder (or write the target folder).`,
        }
      : {
          key: "login",
          title: "Database login and privileges",
          who: "Site",
          state: "manual",
          detail: "A TCP check cannot verify logins. Use Test connection on the profile's connection (Connect through → this tunnel).",
        },
  )
  return steps
}

/** "4 of 6 ready" — skipped steps do not count; manual ones count as not yet confirmed. */
export function readinessSummary(steps: ReadinessStep[]): { done: number; total: number; blocking?: ReadinessStep } {
  const counted = steps.filter((s) => s.state !== "skip")
  return {
    done: counted.filter((s) => s.state === "ok").length,
    total: counted.length,
    blocking: counted.find((s) => s.state === "fail") || counted.find((s) => s.state === "pending" || s.state === "manual"),
  }
}
