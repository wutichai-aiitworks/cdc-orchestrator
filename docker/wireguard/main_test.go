package main

import (
	"bufio"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"strings"
	"testing"
	"time"

	"golang.zx2c4.com/wireguard/conn"
	"golang.zx2c4.com/wireguard/device"
	"golang.zx2c4.com/wireguard/tun/netstack"
)

// RFC 7748 §6.1 X25519 test vector (Alice).
func TestPublicKeyMatchesRFC7748(t *testing.T) {
	priv, _ := hex.DecodeString("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a")
	want := "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"
	if got := hex.EncodeToString(publicKey(priv)); got != want {
		t.Fatalf("public key %s, want %s", got, want)
	}
	k := newPrivateKey()
	if k[0]&7 != 0 || k[31]&128 != 0 || k[31]&64 == 0 {
		t.Fatalf("private key not clamped: %x", k)
	}
}

func TestValidateExplainsMistakes(t *testing.T) {
	k := base64.StdEncoding.EncodeToString(newPrivateKey())
	ok := TunnelSpec{ID: "t", PrivateKey: k, Addresses: []string{"10.66.1.2/32"},
		Peer: PeerSpec{PublicKey: k, Endpoint: "vpn.example:51820", AllowedIPs: []string{"10.66.1.1/32"}}}
	if err := validate(ok); err != nil {
		t.Fatalf("valid spec rejected: %v", err)
	}
	cases := map[string]func(s *TunnelSpec){
		"private_key":      func(s *TunnelSpec) { s.PrivateKey = "nope" },
		"addresses":        func(s *TunnelSpec) { s.Addresses = nil },
		"peer.allowed_ips": func(s *TunnelSpec) { s.Peer.AllowedIPs = []string{"10.66.1.1"} },
		"peer.endpoint":    func(s *TunnelSpec) { s.Peer.Endpoint = "vpn.example" },
	}
	for field, mut := range cases {
		s := ok
		s.Peer.AllowedIPs = append([]string{}, ok.Peer.AllowedIPs...)
		mut(&s)
		if err := validate(s); err == nil || !strings.Contains(err.Error(), field) {
			t.Errorf("%s: got %v", field, err)
		}
	}
}

func TestUapiConfig(t *testing.T) {
	k := base64.StdEncoding.EncodeToString(newPrivateKey())
	s := TunnelSpec{PrivateKey: k, Peer: PeerSpec{PublicKey: k, AllowedIPs: []string{"10.66.1.1/32", "192.168.10.0/24"}}}
	u := uapiConfig(s, "203.0.113.5:51820")
	for _, want := range []string{"endpoint=203.0.113.5:51820", "persistent_keepalive_interval=25", "allowed_ip=192.168.10.0/24", "replace_peers=true"} {
		if !strings.Contains(u, want) {
			t.Errorf("uapi missing %q:\n%s", want, u)
		}
	}
}

// startSite runs a userspace WireGuard "site" on loopback with a TCP echo server inside the tunnel
// (standing in for a database), and returns its public key and UDP port.
func startSite(t *testing.T, sitePriv, gwPub []byte) (*netstack.Net, int) {
	t.Helper()
	tdev, tnet, err := netstack.CreateNetTUN([]netip.Addr{netip.MustParseAddr("10.66.0.1")}, nil, 1420)
	if err != nil {
		t.Fatal(err)
	}
	dev := device.NewDevice(tdev, conn.NewDefaultBind(), device.NewLogger(device.LogLevelError, "[site] "))
	udp, _ := net.ListenPacket("udp", "127.0.0.1:0")
	port := udp.LocalAddr().(*net.UDPAddr).Port
	udp.Close()
	cfg := fmt.Sprintf("private_key=%s\nlisten_port=%d\npublic_key=%s\nallowed_ip=10.66.0.2/32\n",
		hex.EncodeToString(sitePriv), port, hex.EncodeToString(gwPub))
	if err := dev.IpcSet(cfg); err != nil {
		t.Fatal(err)
	}
	if err := dev.Up(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(dev.Close)
	ln, err := tnet.ListenTCP(&net.TCPAddr{IP: net.ParseIP("10.66.0.1"), Port: 6379})
	if err != nil {
		t.Fatal(err)
	}
	go func() {
		for {
			c, err := ln.Accept()
			if err != nil {
				return
			}
			go func() { defer c.Close(); io.Copy(c, c) }() // echo
		}
	}()
	return tnet, port
}

func TestForwardReachesServiceInsideTheTunnel(t *testing.T) {
	sitePriv, gwPriv := newPrivateKey(), newPrivateKey()
	_, sitePort := startSite(t, sitePriv, publicKey(gwPriv))

	g := newGateway()
	g.portMin, g.portMax = 39100, 39199
	spec := TunnelSpec{
		ID: "src", Name: "Source site", Version: "1",
		PrivateKey: base64.StdEncoding.EncodeToString(gwPriv),
		Addresses:  []string{"10.66.0.2/32"},
		Peer: PeerSpec{PublicKey: base64.StdEncoding.EncodeToString(publicKey(sitePriv)),
			Endpoint: fmt.Sprintf("127.0.0.1:%d", sitePort), AllowedIPs: []string{"10.66.0.1/32"}, Keepalive: 1},
	}
	if err := g.upsertTunnel(spec); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { g.deleteTunnel("src") })

	f, err := g.ensureForward("src", "10.66.0.1", 6379)
	if err != nil {
		t.Fatal(err)
	}
	again, _ := g.ensureForward("src", "10.66.0.1", 6379)
	if again.ListenPort != f.ListenPort {
		t.Fatalf("forward not idempotent: %d then %d", f.ListenPort, again.ListenPort)
	}

	c, err := net.DialTimeout("tcp", fmt.Sprintf("127.0.0.1:%d", f.ListenPort), 5*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	r := bufio.NewReader(c)
	echo := func(msg string) {
		t.Helper()
		c.SetDeadline(time.Now().Add(10 * time.Second))
		if _, err := c.Write([]byte(msg + "\n")); err != nil {
			t.Fatal(err)
		}
		got, err := r.ReadString('\n')
		if err != nil || got != msg+"\n" {
			t.Fatalf("echo through tunnel: %q, %v", got, err)
		}
	}
	echo("PING")

	g.mu.Lock()
	st := g.status(g.tunnels["src"])
	g.mu.Unlock()
	if !st.Connected || st.RxBytes == 0 {
		t.Fatalf("expected a handshake and traffic, got %+v", st)
	}

	// Editing the peer (same interface) keeps open connections: a running sync is not cut.
	spec.Version, spec.Peer.Keepalive = "2", 5
	if err := g.upsertTunnel(spec); err != nil {
		t.Fatal(err)
	}
	echo("STILL-THERE")

	// Probe through the HTTP API: open port OK; closed port explains itself.
	srv := httptest.NewServer(g.routes("secret"))
	defer srv.Close()
	probe := func(port int) map[string]any {
		req, _ := http.NewRequest("POST", srv.URL+"/v1/tunnels/src/probe", strings.NewReader(fmt.Sprintf(`{"host":"10.66.0.1","port":%d}`, port)))
		req.Header.Set("Authorization", "Bearer secret")
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		defer resp.Body.Close()
		var out map[string]any
		json.NewDecoder(resp.Body).Decode(&out)
		return out
	}
	if p := probe(6379); p["ok"] != true {
		t.Fatalf("probe open port: %v", p)
	}
	if p := probe(6380); p["ok"] != false || !strings.Contains(fmt.Sprint(p["hint"]), "nothing accepts connections") {
		t.Fatalf("probe closed port: %v", p)
	}
	// No token → 401.
	resp, _ := http.Post(srv.URL+"/v1/keys", "application/json", nil)
	if resp.StatusCode != 401 {
		t.Fatalf("unauthenticated call got %d", resp.StatusCode)
	}

	// Deleting the tunnel closes its forwards.
	g.deleteTunnel("src")
	if _, err := net.DialTimeout("tcp", fmt.Sprintf("127.0.0.1:%d", f.ListenPort), time.Second); err == nil {
		t.Fatal("forward still listening after tunnel delete")
	}
}

// The site-side check before a tunnel exists: an echo on the UDP port proves it is open.
func TestUDPProbeDetectsEchoAndSilence(t *testing.T) {
	echo, err := net.ListenPacket("udp", "127.0.0.1:0") // like: socat UDP4-LISTEN:51820,fork PIPE
	if err != nil {
		t.Fatal(err)
	}
	defer echo.Close()
	go func() {
		buf := make([]byte, 512)
		for {
			n, from, err := echo.ReadFrom(buf)
			if err != nil {
				return
			}
			echo.WriteTo(buf[:n], from)
		}
	}()
	if res := udpProbe(echo.LocalAddr().String(), 3*time.Second); res["ok"] != true || res["echoed"] != true {
		t.Fatalf("echo not detected: %v", res)
	}
	// A listener that does not answer (nc -ul, or a firewall that drops): no echo, a hint.
	quiet, _ := net.ListenPacket("udp", "127.0.0.1:0")
	defer quiet.Close()
	res := udpProbe(quiet.LocalAddr().String(), 1500*time.Millisecond)
	if res["ok"] != false || !strings.Contains(fmt.Sprint(res["hint"]), "No echo came back") {
		t.Fatalf("silence: %v", res)
	}
	if res := udpProbe("no-such-host.invalid:51820", time.Second); res["ok"] != false || !strings.Contains(fmt.Sprint(res["error"]), "cannot resolve") {
		t.Fatalf("bad name: %v", res)
	}
}
