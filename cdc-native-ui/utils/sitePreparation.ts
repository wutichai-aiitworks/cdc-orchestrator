// Before creating a WireGuard tunnel (ADR 0010): what a source/target site collects and prepares —
// above all one open UDP port — with commands per platform and a UDP echo test the gateway runs.
// Unit-tested in tests/unit/sitePreparation.test.ts.

export type HostOs = "debian" | "rhel" | "alpine"
export type HostFirewall = "ufw" | "firewalld" | "nftables" | "iptables" | "none"
export type Edge = "public" | "router" | "aws" | "azure" | "gcp"

export const HOST_OS: { value: HostOs; label: string }[] = [
  { value: "debian", label: "Ubuntu / Debian" },
  { value: "rhel", label: "RHEL / Rocky / Alma" },
  { value: "alpine", label: "Alpine" },
]
export const FIREWALLS: { value: HostFirewall; label: string }[] = [
  { value: "ufw", label: "ufw" },
  { value: "firewalld", label: "firewalld" },
  { value: "nftables", label: "nftables" },
  { value: "iptables", label: "iptables" },
  { value: "none", label: "No host firewall" },
]
export const EDGES: { value: Edge; label: string }[] = [
  { value: "public", label: "Host has a public IP" },
  { value: "router", label: "Office/DC router (NAT)" },
  { value: "aws", label: "AWS security group" },
  { value: "azure", label: "Azure NSG" },
  { value: "gcp", label: "Google Cloud firewall" },
]

export type PrepOptions = { port: number; os: HostOs; firewall: HostFirewall; edge: Edge }
export type InfoItem = { label: string; why: string; example: string }
export type PrepStep = { key: string; title: string; detail: string; commands?: string; optional?: boolean }

export const DEFAULT_PREP: PrepOptions = { port: 51820, os: "debian", firewall: "ufw", edge: "router" }

/** What to ask the site for before creating the tunnel. */
export const INFO_TO_COLLECT: InfoItem[] = [
  { label: "Public address of the site", why: "The gateway dials it; it must be reachable from the internet.", example: "vpn.site.example or 203.0.113.10" },
  { label: "UDP port for WireGuard", why: "The one port the site opens. It must be free on the WireGuard host.", example: "51820" },
  { label: "Where the database runs", why: "On the WireGuard host itself, or on another host (then the site forwards and NATs to it).", example: "Same host / 192.168.10.5" },
  { label: "Database host and port, as seen from the site", why: "What the profile connects to through the tunnel.", example: "192.168.10.5:5432" },
  { label: "Networks already used at the site", why: "Tunnel addresses (10.66.x.x by default) must not clash with them.", example: "192.168.10.0/24, 10.0.0.0/16" },
  { label: "Who runs the site's WireGuard, and when changes are allowed", why: "For the tunnel notes and for the change window.", example: "DC team, weekdays 22:00–24:00" },
  { label: "Site's WireGuard public key (optional)", why: "Only if the site generates its own keys; otherwise Hyper CDC generates them.", example: "44 characters ending in =" },
]

function installCommands(os: HostOs): string {
  switch (os) {
    case "debian":
      return "sudo apt update && sudo apt install -y wireguard wireguard-tools socat"
    case "rhel":
      return "sudo dnf install -y epel-release && sudo dnf install -y wireguard-tools socat"
    case "alpine":
      return "sudo apk add wireguard-tools socat"
  }
}

function hostFirewallCommands(fw: HostFirewall, port: number): string {
  switch (fw) {
    case "ufw":
      return `sudo ufw allow ${port}/udp comment 'Hyper CDC WireGuard'\nsudo ufw status | grep ${port}`
    case "firewalld":
      return `sudo firewall-cmd --permanent --add-port=${port}/udp\nsudo firewall-cmd --reload\nsudo firewall-cmd --list-ports`
    case "nftables":
      return `sudo nft add rule inet filter input udp dport ${port} accept\n# make it permanent in /etc/nftables.conf, then: sudo systemctl reload nftables`
    case "iptables":
      return `sudo iptables -I INPUT -p udp --dport ${port} -j ACCEPT\n# persist: sudo netfilter-persistent save   (Debian)   or   sudo service iptables save   (RHEL)`
    case "none":
      return "# no host firewall: nothing to do on the host"
  }
}

function edgeCommands(edge: Edge, port: number): string {
  switch (edge) {
    case "public":
      return "# the host has a public IP directly: nothing to forward (check any provider firewall too)"
    case "router":
      return `# On the router/firewall in front of the site (vendor UI):\n#   Port forward  UDP ${port}  →  <WireGuard host LAN IP>:${port}\n#   Allow inbound UDP ${port} (optionally only from Hyper CDC's public IP)`
    case "aws":
      return `aws ec2 authorize-security-group-ingress --group-id <sg-id> \\\n  --ip-permissions 'IpProtocol=udp,FromPort=${port},ToPort=${port},IpRanges=[{CidrIp=<hyper-cdc-public-ip>/32,Description="Hyper CDC WireGuard"}]'`
    case "azure":
      return `az network nsg rule create -g <resource-group> --nsg-name <nsg> -n hyper-cdc-wireguard \\\n  --priority 300 --direction Inbound --access Allow --protocol Udp \\\n  --destination-port-ranges ${port} --source-address-prefixes <hyper-cdc-public-ip>`
    case "gcp":
      return `gcloud compute firewall-rules create hyper-cdc-wireguard --direction=INGRESS \\\n  --action=ALLOW --rules=udp:${port} --source-ranges=<hyper-cdc-public-ip>/32 --target-tags=<wireguard-host-tag>`
  }
}

/** The site's steps before the tunnel is created, in order. */
export function preparationSteps(o: PrepOptions): PrepStep[] {
  const p = o.port
  return [
    {
      key: "host",
      title: "Choose the WireGuard host",
      detail:
        "The database server itself, or any Linux host on the same network that can reach it (it then forwards to the database). It needs WireGuard (Linux 5.6+ has it built in) and a stable address the router can forward to.",
    },
    { key: "install", title: "Install WireGuard (and socat for the port test)", detail: "Packages only; nothing is configured yet.", commands: installCommands(o.os) },
    { key: "hostfw", title: `Allow UDP ${p} in the host firewall`, detail: "Inbound on the WireGuard host. Database ports stay closed to the outside.", commands: hostFirewallCommands(o.firewall, p) },
    { key: "edge", title: `Open or forward UDP ${p} at the network edge`, detail: "The router, cloud security group or provider firewall in front of the host. Restrict the source to Hyper CDC's public IP if you can.", commands: edgeCommands(o.edge, p) },
    {
      key: "keys",
      title: "Generate the site's keys yourself (optional)",
      detail: "If the site's private key must never leave the site: generate it there and send only the public key. Otherwise Hyper CDC generates both and shows the private key once in the site configuration.",
      commands: `umask 077\nwg genkey | sudo tee /etc/wireguard/site.key | wg pubkey | sudo tee /etc/wireguard/site.pub\ncat /etc/wireguard/site.pub   # send this (public) key; keep site.key on the host`,
      optional: true,
    },
    {
      key: "udptest",
      title: `Test that UDP ${p} reaches the host`,
      detail: `Run a UDP echo on the port for a minute, then press “Test UDP port” here: the gateway sends a test packet and expects it back. A reply proves the firewall, port-forward and NAT are right. Stop the echo (Ctrl+C) before starting WireGuard on the same port.`,
      commands: `sudo socat -v UDP4-LISTEN:${p},fork PIPE\n# no socat? listen only (you will see the test text, but Hyper CDC cannot confirm):\n# sudo nc -ul ${p}`,
    },
    {
      key: "db",
      title: "Prepare the database (can wait until the tunnel exists)",
      detail:
        "It must listen on an address the WireGuard host reaches and allow the tunnel (the gateway's tunnel IP, or the WireGuard host's LAN IP when it forwards). The tunnel's Readiness panel gives the exact lines per engine and checks them through the tunnel.",
      optional: true,
    },
  ]
}

/** A Markdown handoff for whoever runs the site. */
export function siteChecklistMarkdown(o: PrepOptions, siteName = "<site>"): string {
  const lines = [
    `# Hyper CDC WireGuard — site preparation (${siteName})`,
    "",
    `Hyper CDC reaches the database through a WireGuard tunnel. The site opens **one UDP port (${o.port})**; database ports stay closed.`,
    "",
    "## Information we need",
    ...INFO_TO_COLLECT.map((i) => `- [ ] **${i.label}** — ${i.why} _(e.g. ${i.example})_`),
    "",
    "## Steps",
  ]
  preparationSteps(o).forEach((s, i) => {
    lines.push("", `### ${i + 1}. ${s.title}${s.optional ? " (optional)" : ""}`, "", s.detail)
    if (s.commands) lines.push("", "```bash", s.commands, "```")
  })
  lines.push("", "After this, Hyper CDC creates the tunnel and sends you the complete `wg0.conf` and a checklist.", "")
  return lines.join("\n")
}

export type UdpTestResult = { ok: boolean; echoed?: boolean; ms?: number; resolved?: string; error?: string; hint?: string; sent?: string; endpoint: string }

/** One line for the UDP test result. */
export function udpTestSummary(r: UdpTestResult): { tone: "ok" | "err" | "warn"; text: string } {
  if (r.ok) return { tone: "ok", text: `UDP port open: the echo came back from ${r.resolved || r.endpoint} in ${r.ms} ms.` }
  if (r.error) return { tone: "err", text: r.error }
  return { tone: "warn", text: `No echo from ${r.resolved || r.endpoint}.` }
}
