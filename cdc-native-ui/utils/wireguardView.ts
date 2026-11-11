// WireGuard tunnels page (ADR 0010): pure rules, unit-tested in tests/unit/wireguardView.test.ts.
import type { Tone } from "./transferView"

export type TunnelDraft = {
  name: string
  peer_endpoint: string
  peer_public_key: string
  generate_site_keys: boolean
  preshared_key: string
  generate_preshared_key: boolean
  address: string
  site_address: string
  allowed_ips: string[]
  dns: string[]
  keepalive: number | null
  mtu: number | null
  notes: string
}

/** Next free 10.66.N.0/24 for a new tunnel: gateway .2, site .1 (one /24 per tunnel, no overlaps). */
export function suggestAddresses(existing: { address: string; site_address?: string }[]): { address: string; site_address: string } {
  const used = new Set<number>()
  for (const t of existing) {
    for (const a of [t.address, t.site_address || ""]) {
      const m = a.match(/^10\.66\.(\d{1,3})\./)
      if (m) used.add(Number(m[1]))
    }
  }
  let n = 1
  while (used.has(n) && n < 254) n++
  return { address: `10.66.${n}.2/32`, site_address: `10.66.${n}.1/32` }
}

export function emptyDraft(existing: { address: string; site_address?: string }[]): TunnelDraft {
  const a = suggestAddresses(existing)
  return {
    name: "",
    peer_endpoint: "",
    peer_public_key: "",
    generate_site_keys: true,
    preshared_key: "",
    generate_preshared_key: true,
    address: a.address,
    site_address: a.site_address,
    allowed_ips: [a.site_address],
    dns: [],
    keepalive: 25,
    mtu: null,
    notes: "",
  }
}

const CIDR = /^(\d{1,3}\.){3}\d{1,3}\/\d{1,2}$/
const KEY = /^[A-Za-z0-9+/]{43}=$/

/** What stops a save, in plain words (the server checks the same and more). */
export function draftProblems(d: TunnelDraft, isNew: boolean): string[] {
  const out: string[] = []
  if (!d.name.trim()) out.push("Give the tunnel a name, e.g. “Source site”.")
  if (!/^[^\s:]+:\d{1,5}$/.test(d.peer_endpoint.trim())) out.push("Site endpoint must be host:port of the site's WireGuard (UDP), e.g. vpn.source.example:51820.")
  if (!(isNew && d.generate_site_keys) && !KEY.test(d.peer_public_key.trim())) out.push("Paste the site's WireGuard public key (44 characters, ends with =), or let this page generate the site keys.")
  if (!CIDR.test(d.address.trim())) out.push("This gateway's tunnel address must be an address with a mask, e.g. 10.66.1.2/32.")
  if (d.site_address.trim() && !CIDR.test(d.site_address.trim())) out.push("The site's tunnel address must look like 10.66.1.1/32.")
  if (!d.allowed_ips.length) out.push("List what the tunnel reaches: the site's tunnel address and the database network.")
  for (const a of d.allowed_ips) if (!CIDR.test(a)) out.push(`“${a}” is not a network (use CIDR, e.g. 192.168.10.0/24 or 192.168.10.5/32).`)
  return out
}

/** Status line for a tunnel from the gateway's report. */
export function tunnelStatus(s: { connected: boolean; handshake_age_secs?: number | null } | null | undefined, gatewayReachable = true): { tone: Tone; text: string } {
  if (!gatewayReachable) return { tone: "idle", text: "Gateway unreachable" }
  if (!s) return { tone: "warn", text: "Not on the gateway yet" }
  if (s.handshake_age_secs == null) return { tone: "warn", text: "No handshake yet" }
  const age = s.handshake_age_secs
  const when = age < 60 ? `${age} s ago` : age < 3600 ? `${Math.floor(age / 60)} min ago` : `${Math.floor(age / 3600)} h ago`
  return s.connected ? { tone: "ok", text: `Connected · handshake ${when}` } : { tone: "err", text: `Disconnected · last handshake ${when}` }
}

export function fmtBytes(n: number | null | undefined): string {
  if (!n) return "0 B"
  const u = ["B", "KB", "MB", "GB", "TB"]
  let i = 0
  let v = n
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024
    i++
  }
  return `${v < 10 && i ? v.toFixed(1) : Math.round(v)} ${u[i]}`
}

/** Body for create/update: keeps saved secrets (********) and drops generate flags on edit. */
export function toRequest(d: TunnelDraft, isNew: boolean): Record<string, any> {
  return {
    name: d.name.trim(),
    peer_endpoint: d.peer_endpoint.trim(),
    peer_public_key: isNew && d.generate_site_keys ? "" : d.peer_public_key.trim(),
    generate_site_keys: isNew && d.generate_site_keys,
    preshared_key: d.preshared_key,
    generate_preshared_key: isNew && d.generate_preshared_key,
    address: d.address.trim(),
    site_address: d.site_address.trim(),
    allowed_ips: d.allowed_ips,
    dns: d.dns,
    keepalive: d.keepalive || null,
    mtu: d.mtu || null,
    notes: d.notes.trim(),
  }
}

/** True while the site file still has a placeholder secret (the private key is never stored here). */
export const configHasPlaceholders = (config: string) => /^(PrivateKey|PresharedKey) = </m.test(config)

/**
 * Manual steps for the site's admin, run by hand (nothing installs automatically). No secrets:
 * the private key only travels inside the wg0.conf the admin copies or downloads.
 */
export function siteManualSteps(listenPort: number, iface = "wg0"): { title: string; commands: string }[] {
  return [
    {
      title: "Install WireGuard tools (and ip, which wg-quick needs)",
      commands: [
        "# Ubuntu / Debian",
        "sudo apt-get update && sudo apt-get install -y --no-install-recommends wireguard-tools iproute2",
        "# RHEL / Rocky / Alma:  sudo dnf install -y wireguard-tools iproute",
        "# Alpine:               sudo apk add wireguard-tools iproute2",
      ].join("\n"),
    },
    {
      title: `Save the file as /etc/wireguard/${iface}.conf (owner-only)`,
      commands: [
        `# uploaded ${iface}.conf (e.g. scp to /tmp):`,
        `sudo install -m 600 /tmp/${iface}.conf /etc/wireguard/${iface}.conf && shred -u /tmp/${iface}.conf`,
        "# or paste it:",
        `sudo install -d -m 700 /etc/wireguard && sudo nano /etc/wireguard/${iface}.conf && sudo chmod 600 /etc/wireguard/${iface}.conf`,
      ].join("\n"),
    },
    {
      title: `Stop the UDP echo test if it is still running (it holds UDP ${listenPort})`,
      commands: `# Ctrl+C in the socat / nc window, or:\nsudo pkill -f "UDP4-LISTEN:${listenPort}"`,
    },
    {
      title: "Start the tunnel now and at boot, then check",
      commands: `sudo systemctl enable --now wg-quick@${iface}\nsudo wg show ${iface}    # "latest handshake" = connected`,
    },
  ]
}
