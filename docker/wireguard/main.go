// cdc-wireguard: userspace WireGuard gateway for the Copy & Sync tools (ADR 0010).
//
// Each tunnel is one wireguard-go device on its own gVisor netstack: no TUN device, no
// NET_ADMIN, no host routes. A forward is a TCP listener on this container that dials a
// host:port inside a tunnel; the tools connect to cdc-wireguard:<port> with plain Docker
// networking. Forwards are added while tunnels run and never restart them, so a live CDC
// sync is not cut when another profile starts using the same tunnel.
//
// Secrets stay in memory: the orchestrator owns the tunnel definitions (rqlite, redacted in
// its API) and pushes them again when this process restarts (it compares `version`).
package main

import (
	"context"
	"crypto/rand"
	"crypto/subtle"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"net/netip"
	"os"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"golang.org/x/crypto/curve25519"
	"golang.zx2c4.com/wireguard/conn"
	"golang.zx2c4.com/wireguard/device"
	"golang.zx2c4.com/wireguard/tun/netstack"
)

// ── specs (what the orchestrator sends) ─────────────────────────────────────

type PeerSpec struct {
	PublicKey    string   `json:"public_key"`
	PresharedKey string   `json:"preshared_key,omitempty"`
	Endpoint     string   `json:"endpoint"` // host:port of the site's WireGuard (UDP)
	AllowedIPs   []string `json:"allowed_ips"`
	Keepalive    int      `json:"keepalive,omitempty"`
}

type TunnelSpec struct {
	ID         string   `json:"id"`
	Name       string   `json:"name"`
	Version    string   `json:"version"` // opaque; the orchestrator re-pushes when it differs
	PrivateKey string   `json:"private_key"`
	Addresses  []string `json:"addresses"` // this side's tunnel address(es), e.g. 10.66.1.2/32
	DNS        []string `json:"dns,omitempty"`
	MTU        int      `json:"mtu,omitempty"`
	Peer       PeerSpec `json:"peer"`
}

// ── runtime state ───────────────────────────────────────────────────────────

type tunnel struct {
	spec     TunnelSpec
	dev      *device.Device
	tnet     *netstack.Net
	pubHex   string
	endpoint string // resolved ip:port currently configured
	err      string
}

type forward struct {
	TunnelID   string `json:"tunnel_id"`
	Host       string `json:"host"`
	Port       int    `json:"port"`
	ListenPort int    `json:"listen_port"`
	Conns      int64  `json:"active_connections"`
	ln         net.Listener
}

type gateway struct {
	mu        sync.Mutex
	tunnels   map[string]*tunnel
	forwards  map[string]*forward // tunnelID|host|port
	portMin   int
	portMax   int
	dialWait  time.Duration
	publicHost string
}

func newGateway() *gateway {
	return &gateway{
		tunnels:    map[string]*tunnel{},
		forwards:   map[string]*forward{},
		portMin:    envInt("WG_FORWARD_PORT_MIN", 21000),
		portMax:    envInt("WG_FORWARD_PORT_MAX", 21999),
		dialWait:   time.Duration(envInt("WG_DIAL_TIMEOUT_SECS", 10)) * time.Second,
		publicHost: envStr("WG_FORWARD_HOST", "cdc-wireguard"),
	}
}

// ── keys ────────────────────────────────────────────────────────────────────

func decodeKey(b64 string) ([]byte, error) {
	k, err := base64.StdEncoding.DecodeString(strings.TrimSpace(b64))
	if err != nil || len(k) != 32 {
		return nil, errors.New("must be a base64 WireGuard key (32 bytes)")
	}
	return k, nil
}

func publicKey(priv []byte) []byte {
	pub, _ := curve25519.X25519(priv, curve25519.Basepoint)
	return pub
}

func newPrivateKey() []byte {
	k := make([]byte, 32)
	if _, err := rand.Read(k); err != nil {
		panic(err)
	}
	k[0] &= 248 // clamp, as `wg genkey`
	k[31] = (k[31] & 127) | 64
	return k
}

// ── config → UAPI ───────────────────────────────────────────────────────────

func parseAddrs(list []string, what string) ([]netip.Addr, error) {
	var out []netip.Addr
	for _, s := range list {
		s = strings.TrimSpace(s)
		if s == "" {
			continue
		}
		if p, err := netip.ParsePrefix(s); err == nil {
			out = append(out, p.Addr())
			continue
		}
		a, err := netip.ParseAddr(s)
		if err != nil {
			return nil, fmt.Errorf("%s: %q is not an IP address", what, s)
		}
		out = append(out, a)
	}
	return out, nil
}

func validate(s TunnelSpec) error {
	if s.ID == "" {
		return errors.New("id is required")
	}
	if _, err := decodeKey(s.PrivateKey); err != nil {
		return fmt.Errorf("private_key %v", err)
	}
	if _, err := decodeKey(s.Peer.PublicKey); err != nil {
		return fmt.Errorf("peer.public_key %v", err)
	}
	if s.Peer.PresharedKey != "" {
		if _, err := decodeKey(s.Peer.PresharedKey); err != nil {
			return fmt.Errorf("peer.preshared_key %v", err)
		}
	}
	if addrs, err := parseAddrs(s.Addresses, "addresses"); err != nil {
		return err
	} else if len(addrs) == 0 {
		return errors.New("addresses: this side needs a tunnel address, e.g. 10.66.1.2/32")
	}
	if _, err := parseAddrs(s.DNS, "dns"); err != nil {
		return err
	}
	if len(s.Peer.AllowedIPs) == 0 {
		return errors.New("peer.allowed_ips: list the site networks reachable through the tunnel")
	}
	for _, a := range s.Peer.AllowedIPs {
		if _, err := netip.ParsePrefix(strings.TrimSpace(a)); err != nil {
			return fmt.Errorf("peer.allowed_ips: %q is not a CIDR (e.g. 10.66.1.1/32, 192.168.10.0/24)", a)
		}
	}
	if _, _, err := net.SplitHostPort(s.Peer.Endpoint); err != nil {
		return errors.New("peer.endpoint must be host:port of the site's WireGuard (UDP), e.g. vpn.site.example:51820")
	}
	return nil
}

// resolveEndpoint turns host:port into ip:port (UAPI needs an address).
func resolveEndpoint(ep string) (string, error) {
	a, err := net.ResolveUDPAddr("udp", ep)
	if err != nil {
		return "", fmt.Errorf("cannot resolve the site endpoint %s: %v", ep, err)
	}
	return a.String(), nil
}

func uapiConfig(s TunnelSpec, endpoint string) string {
	priv, _ := decodeKey(s.PrivateKey)
	peer, _ := decodeKey(s.Peer.PublicKey)
	var b strings.Builder
	fmt.Fprintf(&b, "private_key=%s\n", hex.EncodeToString(priv))
	fmt.Fprintf(&b, "replace_peers=true\npublic_key=%s\n", hex.EncodeToString(peer))
	if s.Peer.PresharedKey != "" {
		psk, _ := decodeKey(s.Peer.PresharedKey)
		fmt.Fprintf(&b, "preshared_key=%s\n", hex.EncodeToString(psk))
	}
	fmt.Fprintf(&b, "endpoint=%s\n", endpoint)
	ka := s.Peer.Keepalive
	if ka <= 0 {
		ka = 25 // keep NAT state on the site's side warm; also gives a live handshake signal
	}
	fmt.Fprintf(&b, "persistent_keepalive_interval=%d\nreplace_allowed_ips=true\n", ka)
	for _, a := range s.Peer.AllowedIPs {
		fmt.Fprintf(&b, "allowed_ip=%s\n", strings.TrimSpace(a))
	}
	return b.String()
}

// ── tunnels ─────────────────────────────────────────────────────────────────

func (g *gateway) upsertTunnel(s TunnelSpec) error {
	if err := validate(s); err != nil {
		return err
	}
	endpoint, err := resolveEndpoint(s.Peer.Endpoint)
	if err != nil {
		return err
	}
	addrs, _ := parseAddrs(s.Addresses, "addresses")
	dns, _ := parseAddrs(s.DNS, "dns")
	mtu := s.MTU
	if mtu <= 0 {
		mtu = 1420
	}

	g.mu.Lock()
	old := g.tunnels[s.ID]
	g.mu.Unlock()
	// Same interface (addresses, DNS, MTU, key): reconfigure the peer in place, keeping open
	// connections. Otherwise a new netstack is needed.
	if old != nil && old.dev != nil && sameInterface(old.spec, s) {
		if err := old.dev.IpcSet(uapiConfig(s, endpoint)); err != nil {
			return fmt.Errorf("reconfigure: %v", err)
		}
		g.mu.Lock()
		old.spec, old.endpoint, old.err = s, endpoint, ""
		g.mu.Unlock()
		return nil
	}

	tdev, tnet, err := netstack.CreateNetTUN(addrs, dns, mtu)
	if err != nil {
		return fmt.Errorf("create netstack: %v", err)
	}
	name := s.Name
	if name == "" {
		name = s.ID
	}
	logger := device.NewLogger(device.LogLevelError, fmt.Sprintf("[%s] ", name))
	dev := device.NewDevice(tdev, conn.NewDefaultBind(), logger)
	if err := dev.IpcSet(uapiConfig(s, endpoint)); err != nil {
		dev.Close()
		return fmt.Errorf("configure: %v", err)
	}
	if err := dev.Up(); err != nil {
		dev.Close()
		return fmt.Errorf("up: %v", err)
	}
	priv, _ := decodeKey(s.PrivateKey)
	t := &tunnel{spec: s, dev: dev, tnet: tnet, pubHex: hex.EncodeToString(publicKey(priv)), endpoint: endpoint}
	g.mu.Lock()
	g.tunnels[s.ID] = t
	g.mu.Unlock()
	if old != nil && old.dev != nil {
		old.dev.Close()
	}
	log.Printf("tunnel %q up: %v → %s", name, s.Addresses, s.Peer.Endpoint)
	return nil
}

func sameInterface(a, b TunnelSpec) bool {
	return a.PrivateKey == b.PrivateKey && strings.Join(a.Addresses, ",") == strings.Join(b.Addresses, ",") &&
		strings.Join(a.DNS, ",") == strings.Join(b.DNS, ",") && a.MTU == b.MTU
}

func (g *gateway) deleteTunnel(id string) bool {
	g.mu.Lock()
	t := g.tunnels[id]
	delete(g.tunnels, id)
	var closing []*forward
	for k, f := range g.forwards {
		if f.TunnelID == id {
			closing = append(closing, f)
			delete(g.forwards, k)
		}
	}
	g.mu.Unlock()
	for _, f := range closing {
		f.ln.Close()
	}
	if t != nil && t.dev != nil {
		t.dev.Close()
	}
	return t != nil
}

// reresolve keeps DNS endpoints current (sites with dynamic IPs).
func (g *gateway) reresolve() {
	g.mu.Lock()
	list := make([]*tunnel, 0, len(g.tunnels))
	for _, t := range g.tunnels {
		list = append(list, t)
	}
	g.mu.Unlock()
	for _, t := range list {
		ep, err := resolveEndpoint(t.spec.Peer.Endpoint)
		if err != nil || ep == t.endpoint {
			continue
		}
		peer, _ := decodeKey(t.spec.Peer.PublicKey)
		if err := t.dev.IpcSet(fmt.Sprintf("public_key=%s\nendpoint=%s\n", hex.EncodeToString(peer), ep)); err == nil {
			log.Printf("tunnel %q: site endpoint now %s", t.spec.Name, ep)
			g.mu.Lock()
			t.endpoint = ep
			g.mu.Unlock()
		}
	}
}

type tunnelStatus struct {
	ID              string     `json:"id"`
	Name            string     `json:"name"`
	Version         string     `json:"version"`
	PublicKey       string     `json:"public_key"`
	Endpoint        string     `json:"endpoint"`
	ResolvedEndpoint string    `json:"resolved_endpoint"`
	LastHandshake   *time.Time `json:"last_handshake,omitempty"`
	HandshakeAgeSec *int64     `json:"handshake_age_secs,omitempty"`
	Connected       bool       `json:"connected"`
	RxBytes         int64      `json:"rx_bytes"`
	TxBytes         int64      `json:"tx_bytes"`
	Forwards        []forward  `json:"forwards"`
}

// connected: a handshake within 3 minutes (WireGuard rekeys every 2; keepalive keeps it fresh).
func (g *gateway) status(t *tunnel) tunnelStatus {
	priv, _ := decodeKey(t.spec.PrivateKey)
	st := tunnelStatus{
		ID: t.spec.ID, Name: t.spec.Name, Version: t.spec.Version,
		PublicKey: base64.StdEncoding.EncodeToString(publicKey(priv)),
		Endpoint:  t.spec.Peer.Endpoint, ResolvedEndpoint: t.endpoint, Forwards: []forward{},
	}
	if dump, err := t.dev.IpcGet(); err == nil {
		var sec, nsec int64
		for _, line := range strings.Split(dump, "\n") {
			k, v, ok := strings.Cut(line, "=")
			if !ok {
				continue
			}
			n, _ := strconv.ParseInt(v, 10, 64)
			switch k {
			case "last_handshake_time_sec":
				sec = n
			case "last_handshake_time_nsec":
				nsec = n
			case "rx_bytes":
				st.RxBytes = n
			case "tx_bytes":
				st.TxBytes = n
			}
		}
		if sec > 0 {
			ts := time.Unix(sec, nsec).UTC()
			age := int64(time.Since(ts).Seconds())
			st.LastHandshake, st.HandshakeAgeSec = &ts, &age
			st.Connected = age < 180
		}
	}
	return st
}

// ── forwards ────────────────────────────────────────────────────────────────

func fwdKey(tunnelID, host string, port int) string {
	return fmt.Sprintf("%s|%s|%d", tunnelID, strings.ToLower(host), port)
}

// ensureForward returns the gateway port that reaches host:port through the tunnel (idempotent).
func (g *gateway) ensureForward(tunnelID, host string, port int) (*forward, error) {
	host = strings.TrimSpace(host)
	if host == "" || port <= 0 || port > 65535 {
		return nil, errors.New("host and a port 1-65535 are required")
	}
	g.mu.Lock()
	defer g.mu.Unlock()
	if _, ok := g.tunnels[tunnelID]; !ok {
		return nil, fmt.Errorf("tunnel %s is not configured on the gateway", tunnelID)
	}
	key := fwdKey(tunnelID, host, port)
	if f, ok := g.forwards[key]; ok {
		return f, nil
	}
	used := map[int]bool{}
	for _, f := range g.forwards {
		used[f.ListenPort] = true
	}
	for p := g.portMin; p <= g.portMax; p++ {
		if used[p] {
			continue
		}
		ln, err := net.Listen("tcp", fmt.Sprintf("0.0.0.0:%d", p))
		if err != nil {
			continue // taken by something else
		}
		f := &forward{TunnelID: tunnelID, Host: host, Port: port, ListenPort: p, ln: ln}
		g.forwards[key] = f
		go g.serve(f)
		log.Printf("forward :%d → %s:%d via tunnel %s", p, host, port, tunnelID)
		return f, nil
	}
	return nil, fmt.Errorf("no free forward port in %d-%d", g.portMin, g.portMax)
}

func (g *gateway) dial(ctx context.Context, tunnelID, host string, port int) (net.Conn, error) {
	g.mu.Lock()
	t := g.tunnels[tunnelID]
	g.mu.Unlock()
	if t == nil {
		return nil, fmt.Errorf("tunnel %s is not configured", tunnelID)
	}
	ctx, cancel := context.WithTimeout(ctx, g.dialWait)
	defer cancel()
	c, err := t.tnet.DialContext(ctx, "tcp", net.JoinHostPort(host, strconv.Itoa(port)))
	if err != nil && net.ParseIP(host) == nil && len(t.spec.DNS) == 0 {
		return nil, fmt.Errorf("%v (the host %q is a name; set the tunnel's DNS to a resolver at the site, or use an IP)", err, host)
	}
	return c, err
}

func (g *gateway) serve(f *forward) {
	for {
		c, err := f.ln.Accept()
		if err != nil {
			return // listener closed (tunnel deleted)
		}
		go func(client net.Conn) {
			defer client.Close()
			remote, err := g.dial(context.Background(), f.TunnelID, f.Host, f.Port)
			if err != nil {
				log.Printf("forward :%d → %s:%d: %v", f.ListenPort, f.Host, f.Port, err)
				return
			}
			defer remote.Close()
			g.mu.Lock()
			f.Conns++
			g.mu.Unlock()
			defer func() { g.mu.Lock(); f.Conns--; g.mu.Unlock() }()
			done := make(chan struct{}, 2)
			go func() { io.Copy(remote, client); closeWrite(remote); done <- struct{}{} }()
			go func() { io.Copy(client, remote); closeWrite(client); done <- struct{}{} }()
			<-done
			<-done
		}(c)
	}
}

func closeWrite(c net.Conn) {
	if cw, ok := c.(interface{ CloseWrite() error }); ok {
		cw.CloseWrite()
	}
}

// ── HTTP API ────────────────────────────────────────────────────────────────

func (g *gateway) routes(token string) http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /health", func(w http.ResponseWriter, r *http.Request) {
		g.mu.Lock()
		n := len(g.tunnels)
		g.mu.Unlock()
		writeJSON(w, 200, map[string]any{"status": "ok", "service": "cdc-wireguard", "tunnels": n})
	})
	mux.HandleFunc("GET /v1/capability", func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, 200, map[string]any{"engine": "wireguard-go (userspace netstack)", "forward_host": g.publicHost,
			"forward_ports": fmt.Sprintf("%d-%d", g.portMin, g.portMax)})
	})
	mux.HandleFunc("POST /v1/keys", func(w http.ResponseWriter, r *http.Request) {
		var body struct {
			PrivateKey string `json:"private_key"`
		}
		_ = json.NewDecoder(r.Body).Decode(&body)
		priv := newPrivateKey()
		if body.PrivateKey != "" { // derive the public key of a given private key
			k, err := decodeKey(body.PrivateKey)
			if err != nil {
				writeErr(w, 400, "private_key "+err.Error())
				return
			}
			priv = k
		}
		writeJSON(w, 200, map[string]string{
			"private_key": base64.StdEncoding.EncodeToString(priv),
			"public_key":  base64.StdEncoding.EncodeToString(publicKey(priv)),
		})
	})
	mux.HandleFunc("POST /v1/keys/preshared", func(w http.ResponseWriter, r *http.Request) {
		k := make([]byte, 32)
		_, _ = rand.Read(k)
		writeJSON(w, 200, map[string]string{"preshared_key": base64.StdEncoding.EncodeToString(k)})
	})
	mux.HandleFunc("GET /v1/tunnels", func(w http.ResponseWriter, r *http.Request) {
		g.mu.Lock()
		list := make([]*tunnel, 0, len(g.tunnels))
		for _, t := range g.tunnels {
			list = append(list, t)
		}
		fw := make([]forward, 0, len(g.forwards))
		for _, f := range g.forwards {
			fw = append(fw, *f)
		}
		g.mu.Unlock()
		out := make([]tunnelStatus, 0, len(list))
		for _, t := range list {
			st := g.status(t)
			for _, f := range fw {
				if f.TunnelID == t.spec.ID {
					f.ln = nil
					st.Forwards = append(st.Forwards, f)
				}
			}
			sort.Slice(st.Forwards, func(i, j int) bool { return st.Forwards[i].ListenPort < st.Forwards[j].ListenPort })
			out = append(out, st)
		}
		sort.Slice(out, func(i, j int) bool { return out[i].Name < out[j].Name })
		writeJSON(w, 200, map[string]any{"tunnels": out})
	})
	mux.HandleFunc("PUT /v1/tunnels/{id}", func(w http.ResponseWriter, r *http.Request) {
		var s TunnelSpec
		if err := json.NewDecoder(io.LimitReader(r.Body, 1<<20)).Decode(&s); err != nil {
			writeErr(w, 400, "invalid JSON: "+err.Error())
			return
		}
		s.ID = r.PathValue("id")
		if err := g.upsertTunnel(s); err != nil {
			writeErr(w, 400, err.Error())
			return
		}
		g.mu.Lock()
		t := g.tunnels[s.ID]
		g.mu.Unlock()
		writeJSON(w, 200, g.status(t))
	})
	mux.HandleFunc("DELETE /v1/tunnels/{id}", func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, 200, map[string]any{"deleted": g.deleteTunnel(r.PathValue("id"))})
	})
	mux.HandleFunc("POST /v1/forwards", func(w http.ResponseWriter, r *http.Request) {
		var body struct {
			TunnelID string `json:"tunnel_id"`
			Host     string `json:"host"`
			Port     int    `json:"port"`
		}
		if err := json.NewDecoder(io.LimitReader(r.Body, 1<<16)).Decode(&body); err != nil {
			writeErr(w, 400, "invalid JSON")
			return
		}
		f, err := g.ensureForward(body.TunnelID, body.Host, body.Port)
		if err != nil {
			writeErr(w, 400, err.Error())
			return
		}
		writeJSON(w, 200, map[string]any{"host": g.publicHost, "port": f.ListenPort, "tunnel_id": f.TunnelID,
			"target": net.JoinHostPort(f.Host, strconv.Itoa(f.Port))})
	})
	// Before a tunnel exists: is the site's UDP port open? (see udpProbe)
	mux.HandleFunc("POST /v1/udp-probe", func(w http.ResponseWriter, r *http.Request) {
		var body struct {
			Endpoint string `json:"endpoint"`
		}
		if err := json.NewDecoder(io.LimitReader(r.Body, 1<<16)).Decode(&body); err != nil {
			writeErr(w, 400, "invalid JSON")
			return
		}
		if _, _, err := net.SplitHostPort(strings.TrimSpace(body.Endpoint)); err != nil {
			writeErr(w, 400, "endpoint must be host:port, e.g. vpn.site.example:51820")
			return
		}
		writeJSON(w, 200, udpProbe(strings.TrimSpace(body.Endpoint), 4500*time.Millisecond))
	})
	// Can this side reach host:port through the tunnel? (TCP connect; a normal answer when it fails.)
	mux.HandleFunc("POST /v1/tunnels/{id}/probe", func(w http.ResponseWriter, r *http.Request) {
		var body struct {
			Host string `json:"host"`
			Port int    `json:"port"`
		}
		if err := json.NewDecoder(io.LimitReader(r.Body, 1<<16)).Decode(&body); err != nil || body.Host == "" || body.Port <= 0 {
			writeErr(w, 400, "host and port are required")
			return
		}
		id := r.PathValue("id")
		g.mu.Lock()
		t := g.tunnels[id]
		g.mu.Unlock()
		if t == nil {
			writeErr(w, 404, "tunnel not configured on the gateway")
			return
		}
		start := time.Now()
		c, err := g.dial(r.Context(), id, body.Host, body.Port)
		st := g.status(t)
		res := map[string]any{"target": net.JoinHostPort(body.Host, strconv.Itoa(body.Port)), "connected": st.Connected,
			"handshake_age_secs": st.HandshakeAgeSec}
		if err != nil {
			res["ok"] = false
			res["error"] = err.Error()
			res["hint"] = probeHint(st, err)
		} else {
			c.Close()
			res["ok"] = true
			res["ms"] = time.Since(start).Milliseconds()
		}
		writeJSON(w, 200, res)
	})
	return auth(token, mux)
}

// udpProbe sends test datagrams to a site's (future) WireGuard UDP port and waits for an echo. Used
// before a tunnel exists: the site runs a UDP echo on the port (socat … PIPE) and a reply proves the
// port is open end to end (firewall, router port-forward, NAT). No reply is not proof of a block if
// the site only listens without echoing (nc -ul): then it sees the text on its side.
func udpProbe(endpoint string, wait time.Duration) map[string]any {
	res := map[string]any{"endpoint": endpoint}
	addr, err := net.ResolveUDPAddr("udp", endpoint)
	if err != nil {
		res["ok"] = false
		res["error"] = fmt.Sprintf("cannot resolve %s: %v", endpoint, err)
		res["hint"] = "Use the site's public IP, or fix the DNS name (A record) of its endpoint."
		return res
	}
	res["resolved"] = addr.String()
	conn, err := net.DialUDP("udp", nil, addr)
	if err != nil {
		res["ok"] = false
		res["error"] = err.Error()
		return res
	}
	defer conn.Close()
	token := make([]byte, 4)
	_, _ = rand.Read(token)
	payload := fmt.Sprintf("hyper-cdc-udp-test %x\n", token)
	start := time.Now()
	buf := make([]byte, 512)
	// A few tries: the first datagram can be dropped while NAT state is created.
	for try := 0; try < 3; try++ {
		if _, err := conn.Write([]byte(payload)); err != nil {
			res["ok"] = false
			res["error"] = err.Error()
			return res
		}
		_ = conn.SetReadDeadline(time.Now().Add(wait / 3))
		n, err := conn.Read(buf)
		if err == nil && strings.Contains(string(buf[:n]), fmt.Sprintf("%x", token)) {
			res["ok"] = true
			res["echoed"] = true
			res["ms"] = time.Since(start).Milliseconds()
			return res
		}
		if err != nil && strings.Contains(err.Error(), "refused") {
			res["ok"] = false
			res["error"] = "the site answered that nothing listens on this UDP port (ICMP port unreachable)"
			res["hint"] = "Packets reach the host, but no echo/WireGuard is listening on that port there. Start the echo command (or WireGuard) on the right port."
			return res
		}
	}
	res["ok"] = false
	res["echoed"] = false
	res["sent"] = payload[:len(payload)-1]
	res["hint"] = "No echo came back. If the site runs the socat echo, the UDP port is still blocked (host firewall, cloud security group, router port-forward) or the endpoint address is wrong. If it only listens with nc -ul, ask whether it printed the text above."
	return res
}

func probeHint(st tunnelStatus, err error) string {
	if st.LastHandshake == nil {
		return "No WireGuard handshake with the site yet: check that the site's UDP port is open, the endpoint is right, and the site lists this gateway's public key as a peer."
	}
	if strings.Contains(err.Error(), "refused") {
		return "The tunnel works but nothing accepts connections at that address: check the database listens on it (bind address) and its firewall allows the tunnel network."
	}
	return "The tunnel is up but the address did not answer: check the site's AllowedIPs/routing (IP forwarding and NAT to the database host) and the database's allow-list."
}

func auth(token string, next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if token != "" && r.URL.Path != "/health" {
			got := strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer ")
			if subtle.ConstantTimeCompare([]byte(got), []byte(token)) != 1 {
				writeErr(w, 401, "invalid or missing bearer token")
				return
			}
		}
		next.ServeHTTP(w, r)
	})
}

func writeJSON(w http.ResponseWriter, code int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(code)
	_ = json.NewEncoder(w).Encode(v)
}

func writeErr(w http.ResponseWriter, code int, msg string) {
	writeJSON(w, code, map[string]string{"error": msg})
}

func envStr(k, def string) string {
	if v := strings.TrimSpace(os.Getenv(k)); v != "" {
		return v
	}
	return def
}

func envInt(k string, def int) int {
	if n, err := strconv.Atoi(os.Getenv(k)); err == nil {
		return n
	}
	return def
}

func main() {
	g := newGateway()
	go func() {
		for range time.Tick(5 * time.Minute) {
			g.reresolve()
		}
	}()
	addr := fmt.Sprintf("%s:%d", envStr("WG_API_HOST", "0.0.0.0"), envInt("WG_API_PORT", 8091))
	token := envStr("WIREGUARD_API_TOKEN", "")
	if token == "" {
		log.Printf("WIREGUARD_API_TOKEN is not set: the API accepts any caller on the Docker network")
	}
	log.Printf("cdc-wireguard listening on %s (forwards %d-%d as %s)", addr, g.portMin, g.portMax, g.publicHost)
	log.Fatal(http.ListenAndServe(addr, g.routes(token)))
}
