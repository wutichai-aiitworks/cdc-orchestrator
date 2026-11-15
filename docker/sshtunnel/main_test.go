package main

import (
	"bufio"
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"encoding/binary"
	"encoding/pem"
	"fmt"
	"io"
	"net"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"golang.org/x/crypto/ssh"
)

// ── test SSH server (direct-tcpip only, like `restrict,port-forwarding`) ──────

type testSSHD struct {
	ln       net.Listener
	addr     string
	hostFP   string
	mu       sync.Mutex
	conns    []net.Conn
	deny     bool // refuse port forwards
	host     ssh.Signer
	accepted int
}

func newTestSSHD(t *testing.T, userKey ssh.PublicKey, addr string) *testSSHD {
	t.Helper()
	_, hostPriv, _ := ed25519.GenerateKey(rand.Reader)
	hostSigner, _ := ssh.NewSignerFromKey(hostPriv)
	return startSSHD(t, userKey, addr, hostSigner)
}

func startSSHD(t *testing.T, userKey ssh.PublicKey, addr string, hostSigner ssh.Signer) *testSSHD {
	t.Helper()
	cfg := &ssh.ServerConfig{
		PublicKeyCallback: func(_ ssh.ConnMetadata, k ssh.PublicKey) (*ssh.Permissions, error) {
			if string(k.Marshal()) == string(userKey.Marshal()) {
				return nil, nil
			}
			return nil, fmt.Errorf("unknown key")
		},
	}
	cfg.AddHostKey(hostSigner)
	if addr == "" {
		addr = "127.0.0.1:0"
	}
	ln, err := net.Listen("tcp", addr)
	if err != nil {
		t.Fatal(err)
	}
	s := &testSSHD{ln: ln, addr: ln.Addr().String(), hostFP: ssh.FingerprintSHA256(hostSigner.PublicKey()), host: hostSigner}
	go func() {
		for {
			c, err := ln.Accept()
			if err != nil {
				return
			}
			s.mu.Lock()
			s.conns = append(s.conns, c)
			s.accepted++
			s.mu.Unlock()
			go s.handle(c, cfg)
		}
	}()
	return s
}

func (s *testSSHD) handle(c net.Conn, cfg *ssh.ServerConfig) {
	_, chans, reqs, err := ssh.NewServerConn(c, cfg)
	if err != nil {
		return
	}
	go func() {
		for r := range reqs {
			if r.WantReply {
				r.Reply(r.Type == "keepalive@openssh.com", nil)
			}
		}
	}()
	for nc := range chans {
		if nc.ChannelType() != "direct-tcpip" || s.deny {
			nc.Reject(ssh.Prohibited, "administratively prohibited")
			continue
		}
		// payload: host string, port uint32, origin string, origin port uint32
		p := nc.ExtraData()
		hl := binary.BigEndian.Uint32(p[:4])
		host := string(p[4 : 4+hl])
		port := binary.BigEndian.Uint32(p[4+hl : 8+hl])
		target, err := net.Dial("tcp", net.JoinHostPort(host, strconv.Itoa(int(port))))
		if err != nil {
			nc.Reject(ssh.ConnectionFailed, "connect refused")
			continue
		}
		ch, creqs, err := nc.Accept()
		if err != nil {
			target.Close()
			continue
		}
		go ssh.DiscardRequests(creqs)
		go func() {
			defer ch.Close()
			defer target.Close()
			done := make(chan struct{}, 2)
			go func() { io.Copy(target, ch); done <- struct{}{} }()
			go func() { io.Copy(ch, target); done <- struct{}{} }()
			<-done
		}()
	}
}

func (s *testSSHD) stop() {
	s.ln.Close()
	s.mu.Lock()
	for _, c := range s.conns {
		c.Close()
	}
	s.conns = nil
	s.mu.Unlock()
}

func echoServer(t *testing.T) (string, int) {
	t.Helper()
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { ln.Close() })
	go func() {
		for {
			c, err := ln.Accept()
			if err != nil {
				return
			}
			go func() { defer c.Close(); io.Copy(c, c) }()
		}
	}()
	a := ln.Addr().(*net.TCPAddr)
	return "127.0.0.1", a.Port
}

func userKey(t *testing.T) (string, ssh.PublicKey) {
	t.Helper()
	pub, priv, _ := ed25519.GenerateKey(rand.Reader)
	blk, err := ssh.MarshalPrivateKey(priv, "")
	if err != nil {
		t.Fatal(err)
	}
	sp, _ := ssh.NewPublicKey(pub)
	return string(pem.EncodeToMemory(blk)), sp
}

func freePort(t *testing.T) int {
	ln, _ := net.Listen("tcp", "127.0.0.1:0")
	p := ln.Addr().(*net.TCPAddr).Port
	ln.Close()
	return p
}

func testGateway(t *testing.T) *gateway {
	g := newGateway()
	g.portMin, g.portMax = 1024, 65535
	g.dialWait = 5 * time.Second
	g.listen = func(addr string) (net.Listener, error) {
		return net.Listen("tcp", strings.Replace(addr, "0.0.0.0", "127.0.0.1", 1))
	}
	t.Cleanup(func() {
		g.mu.Lock()
		ids := []string{}
		for id := range g.tunnels {
			ids = append(ids, id)
		}
		g.mu.Unlock()
		for _, id := range ids {
			g.remove(id)
		}
	})
	return g
}

func spec(sshd *testSSHD, key string, fwd ...ForwardSpec) TunnelSpec {
	h, p, _ := net.SplitHostPort(sshd.addr)
	port, _ := strconv.Atoi(p)
	return TunnelSpec{ID: "t1", Name: "site-a", Version: "1", Host: h, Port: port, Username: "cdc",
		PrivateKey: key, HostKeyFingerprint: sshd.hostFP, Keepalive: 1, Forwards: fwd}
}

func waitFor(t *testing.T, what string, cond func() bool) {
	t.Helper()
	deadline := time.Now().Add(10 * time.Second)
	for time.Now().Before(deadline) {
		if cond() {
			return
		}
		time.Sleep(50 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for %s", what)
}

func roundTrip(addr string) error {
	c, err := net.DialTimeout("tcp", addr, 2*time.Second)
	if err != nil {
		return err
	}
	defer c.Close()
	_ = c.SetDeadline(time.Now().Add(3 * time.Second))
	if _, err := c.Write([]byte("ping\n")); err != nil {
		return err
	}
	line, err := bufio.NewReader(c).ReadString('\n')
	if err != nil {
		return err
	}
	if line != "ping\n" {
		return fmt.Errorf("echo %q", line)
	}
	return nil
}

func connected(g *gateway, id string) bool {
	g.mu.Lock()
	defer g.mu.Unlock()
	t := g.tunnels[id]
	return t != nil && t.connected
}

// ── tests ───────────────────────────────────────────────────────────────────

func TestForwardRoundTrip(t *testing.T) {
	key, pub := userKey(t)
	sshd := newTestSSHD(t, pub, "")
	defer sshd.stop()
	eh, ep := echoServer(t)
	lp := freePort(t)
	g := testGateway(t)
	if err := g.upsert(spec(sshd, key, ForwardSpec{RemoteHost: eh, RemotePort: ep, ListenPort: lp})); err != nil {
		t.Fatal(err)
	}
	waitFor(t, "connected", func() bool { return connected(g, "t1") })
	if err := roundTrip(fmt.Sprintf("127.0.0.1:%d", lp)); err != nil {
		t.Fatal(err)
	}
}

func TestAddForwardKeepsSession(t *testing.T) {
	key, pub := userKey(t)
	sshd := newTestSSHD(t, pub, "")
	defer sshd.stop()
	eh, ep := echoServer(t)
	lp1, lp2 := freePort(t), freePort(t)
	g := testGateway(t)
	s := spec(sshd, key, ForwardSpec{RemoteHost: eh, RemotePort: ep, ListenPort: lp1})
	if err := g.upsert(s); err != nil {
		t.Fatal(err)
	}
	waitFor(t, "connected", func() bool { return connected(g, "t1") })
	s.Version = "2"
	s.Forwards = append(s.Forwards, ForwardSpec{RemoteHost: eh, RemotePort: ep, ListenPort: lp2})
	if err := g.upsert(s); err != nil {
		t.Fatal(err)
	}
	if err := roundTrip(fmt.Sprintf("127.0.0.1:%d", lp2)); err != nil {
		t.Fatal(err)
	}
	sshd.mu.Lock()
	n := sshd.accepted
	sshd.mu.Unlock()
	if n != 1 {
		t.Fatalf("adding a forward opened %d SSH sessions, want 1", n)
	}
}

func TestListenerClosedWhileDownAndSamePortAfterReconnect(t *testing.T) {
	key, pub := userKey(t)
	sshd := newTestSSHD(t, pub, "")
	eh, ep := echoServer(t)
	lp := freePort(t)
	g := testGateway(t)
	if err := g.upsert(spec(sshd, key, ForwardSpec{RemoteHost: eh, RemotePort: ep, ListenPort: lp})); err != nil {
		t.Fatal(err)
	}
	waitFor(t, "connected", func() bool { return connected(g, "t1") })
	addr := sshd.addr
	sshd.stop()
	waitFor(t, "disconnected", func() bool { return !connected(g, "t1") })
	if c, err := net.DialTimeout("tcp", fmt.Sprintf("127.0.0.1:%d", lp), time.Second); err == nil {
		c.Close()
		t.Fatal("forward port still accepts while the SSH session is down")
	}
	// Same server address and host key comes back.
	sshd2 := startSSHD(t, pub, addr, sshd.host)
	defer sshd2.stop()
	g.mu.Lock()
	select {
	case g.tunnels["t1"].kick <- struct{}{}:
	default:
	}
	g.mu.Unlock()
	waitFor(t, "reconnected", func() bool { return connected(g, "t1") })
	if err := roundTrip(fmt.Sprintf("127.0.0.1:%d", lp)); err != nil {
		t.Fatal(err)
	}
}

func TestProbeUnpinnedReturnsFingerprintWithoutAuth(t *testing.T) {
	key, pub := userKey(t)
	sshd := newTestSSHD(t, pub, "")
	defer sshd.stop()
	g := testGateway(t)
	s := spec(sshd, key)
	s.HostKeyFingerprint = ""
	res := g.probeSpec(context.Background(), s, "", 0)
	if res["ok"] != false || res["needs_host_key"] != true || res["host_key_fingerprint"] != sshd.hostFP {
		t.Fatalf("unexpected probe result %v", res)
	}
}

func TestHostKeyMismatchRefused(t *testing.T) {
	key, pub := userKey(t)
	sshd := newTestSSHD(t, pub, "")
	defer sshd.stop()
	g := testGateway(t)
	s := spec(sshd, key)
	s.HostKeyFingerprint = "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
	res := g.probeSpec(context.Background(), s, "", 0)
	if res["ok"] != false || res["host_key_mismatch"] != true {
		t.Fatalf("unexpected probe result %v", res)
	}
	if err := g.upsert(TunnelSpec{ID: "x", Username: "u", Password: "p", Host: "h"}); err == nil {
		t.Fatal("upsert without a pinned host key must fail")
	}
}

func TestProbeDialAndForwardRefused(t *testing.T) {
	key, pub := userKey(t)
	sshd := newTestSSHD(t, pub, "")
	defer sshd.stop()
	eh, ep := echoServer(t)
	g := testGateway(t)
	res := g.probeSpec(context.Background(), spec(sshd, key), eh, ep)
	if res["ok"] != true {
		t.Fatalf("probe failed: %v", res)
	}
	sshd.deny = true
	res = g.probeSpec(context.Background(), spec(sshd, key), eh, ep)
	if res["ok"] != false || !strings.Contains(fmt.Sprint(res["hint"]), "AllowTcpForwarding") {
		t.Fatalf("expected a forwarding hint, got %v", res)
	}
}

func TestListenPortOwnedByOneTunnel(t *testing.T) {
	g := testGateway(t)
	a := TunnelSpec{ID: "a", Name: "a", Host: "192.0.2.1", Username: "u", Password: "p", HostKeyFingerprint: "SHA256:x",
		Forwards: []ForwardSpec{{RemoteHost: "db", RemotePort: 5432, ListenPort: 22001}}}
	if err := g.upsert(a); err != nil {
		t.Fatal(err)
	}
	b := a
	b.ID, b.Name = "b", "b"
	if err := g.upsert(b); err == nil || !strings.Contains(err.Error(), "already used") {
		t.Fatalf("expected port conflict, got %v", err)
	}
}
