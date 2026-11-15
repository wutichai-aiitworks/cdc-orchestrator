// cdc-sshtunnel: SSH tunnel gateway for sources/targets whose database port is not exposed
// (ADR 0020). Typical case: the site publishes only an SSH app through Cloudflare Zero Trust.
//
// One SSH client per tunnel (golang.org/x/crypto/ssh). A forward is a TCP listener on this
// container that dials remote_host:remote_port through the SSH connection (`ssh -L`). Kafka
// Connect, jdbc-admin, the orchestrator and the Copy & Sync sidecars reach it as
// cdc-sshtunnel:<listen_port> with plain Docker networking.
//
// Unlike cdc-wireguard, forwards are declared by the orchestrator with a fixed listen_port and
// pushed with the tunnel: a Debezium connector stores host:port in its config, so the port must
// be the same after any restart of this process.
//
// Listeners are open only while the SSH session is up. A caller (or the orchestrator's
// auto-recover port check) then sees "connection refused" instead of a port that accepts and
// immediately drops.
//
// Cloudflare Access: the SSH transport is the stdin/stdout of `cloudflared access ssh
// --hostname H` (what ProxyCommand does), with a service token in the child's environment.
//
// Secrets stay in memory: the orchestrator owns tunnel definitions (rqlite, redacted in its API)
// and pushes them again when this process restarts (it compares `version`).
package main

import (
	"context"
	"crypto/subtle"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"os"
	"os/exec"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"golang.org/x/crypto/ssh"
)

// ── specs (what the orchestrator sends) ─────────────────────────────────────

type ProxySpec struct {
	Kind               string `json:"kind,omitempty"` // "" | "none" | "cloudflare_access"
	Hostname           string `json:"hostname,omitempty"`
	ServiceTokenID     string `json:"service_token_id,omitempty"`
	ServiceTokenSecret string `json:"service_token_secret,omitempty"`
}

type ForwardSpec struct {
	RemoteHost string `json:"remote_host"`
	RemotePort int    `json:"remote_port"`
	ListenPort int    `json:"listen_port"`
}

type TunnelSpec struct {
	ID                 string        `json:"id"`
	Name               string        `json:"name"`
	Version            string        `json:"version"` // opaque; the orchestrator re-pushes when it differs
	Host               string        `json:"host"`    // SSH server (or the Access hostname with a Cloudflare proxy)
	Port               int           `json:"port,omitempty"`
	Username           string        `json:"username"`
	PrivateKey         string        `json:"private_key,omitempty"`
	Passphrase         string        `json:"passphrase,omitempty"`
	Password           string        `json:"password,omitempty"`
	HostKeyFingerprint string        `json:"host_key_fingerprint,omitempty"` // SHA256:… (ssh-keygen -lf)
	Keepalive          int           `json:"keepalive,omitempty"`            // seconds, default 15
	Proxy              ProxySpec     `json:"proxy"`
	Forwards           []ForwardSpec `json:"forwards"`
}

// connKey covers every field that needs a new SSH session when it changes. Name, version and
// forwards are not in it: adding a forward must never cut a running CDC stream.
func (s TunnelSpec) connKey() string {
	b, _ := json.Marshal([]any{s.Host, s.port(), s.Username, s.PrivateKey, s.Passphrase, s.Password,
		normFP(s.HostKeyFingerprint), s.Proxy, s.keepalive()})
	return string(b)
}

func (s TunnelSpec) port() int {
	if s.Port > 0 {
		return s.Port
	}
	return 22
}

func (s TunnelSpec) keepalive() time.Duration {
	if s.Keepalive > 0 {
		return time.Duration(s.Keepalive) * time.Second
	}
	return 15 * time.Second
}

func normFP(fp string) string {
	fp = strings.TrimSpace(fp)
	if fp != "" && !strings.HasPrefix(fp, "SHA256:") {
		fp = "SHA256:" + fp
	}
	return strings.TrimRight(fp, "=")
}

func validateSpec(s TunnelSpec, needPin bool) error {
	switch {
	case strings.TrimSpace(s.ID) == "":
		return errors.New("id is required")
	case strings.TrimSpace(s.Username) == "":
		return errors.New("username is required")
	case s.PrivateKey == "" && s.Password == "":
		return errors.New("a private key or a password is required")
	case s.Port < 0 || s.Port > 65535:
		return errors.New("port must be 1-65535")
	case needPin && normFP(s.HostKeyFingerprint) == "":
		return errors.New("host_key_fingerprint is required (test the connection to read it)")
	}
	switch s.Proxy.Kind {
	case "", "none":
		if strings.TrimSpace(s.Host) == "" {
			return errors.New("host is required")
		}
	case "cloudflare_access":
		if strings.TrimSpace(s.Proxy.Hostname) == "" && strings.TrimSpace(s.Host) == "" {
			return errors.New("the Cloudflare Access hostname is required")
		}
	default:
		return fmt.Errorf("unknown proxy kind %q", s.Proxy.Kind)
	}
	seen := map[int]bool{}
	for _, f := range s.Forwards {
		if strings.TrimSpace(f.RemoteHost) == "" || f.RemotePort <= 0 || f.RemotePort > 65535 {
			return errors.New("each forward needs remote_host and remote_port 1-65535")
		}
		if f.ListenPort <= 0 || f.ListenPort > 65535 {
			return errors.New("each forward needs a listen_port")
		}
		if seen[f.ListenPort] {
			return fmt.Errorf("listen_port %d is used twice", f.ListenPort)
		}
		seen[f.ListenPort] = true
	}
	return nil
}

// ── runtime state ───────────────────────────────────────────────────────────

type forward struct {
	spec  ForwardSpec
	ln    net.Listener
	conns map[net.Conn]struct{}
}

type tunnel struct {
	spec       TunnelSpec
	client     *ssh.Client
	connected  bool
	since      time.Time
	lastErr    string
	errAt      time.Time
	observedFP string
	forwards   map[int]*forward // listen_port
	cancel     context.CancelFunc
	kick       chan struct{} // wake the reconnect loop early
}

type gateway struct {
	mu          sync.Mutex
	tunnels     map[string]*tunnel
	publicHost  string
	portMin     int
	portMax     int
	dialWait    time.Duration
	cloudflared string
	listen      func(addr string) (net.Listener, error) // swapped in tests
}

func newGateway() *gateway {
	return &gateway{
		tunnels:     map[string]*tunnel{},
		publicHost:  envStr("SSHT_FORWARD_HOST", "cdc-sshtunnel"),
		portMin:     envInt("SSHT_FORWARD_PORT_MIN", 22000),
		portMax:     envInt("SSHT_FORWARD_PORT_MAX", 22999),
		dialWait:    time.Duration(envInt("SSHT_DIAL_TIMEOUT_SECS", 20)) * time.Second,
		cloudflared: envStr("SSHT_CLOUDFLARED", "cloudflared"),
		listen:      func(addr string) (net.Listener, error) { return net.Listen("tcp", addr) },
	}
}

// ── SSH session ─────────────────────────────────────────────────────────────

type hostKeyError struct{ observed, pinned string }

// hostKeyAlgorithms prefers ed25519 like modern OpenSSH, so the fingerprint matches what operators
// read with `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub`.
var hostKeyAlgorithms = []string{
	ssh.KeyAlgoED25519,
	ssh.KeyAlgoECDSA256, ssh.KeyAlgoECDSA384, ssh.KeyAlgoECDSA521,
	ssh.KeyAlgoRSASHA512, ssh.KeyAlgoRSASHA256, ssh.KeyAlgoRSA,
}

func (e *hostKeyError) Error() string {
	if e.pinned == "" {
		return "host key not pinned; observed " + e.observed
	}
	return fmt.Sprintf("host key mismatch: server sent %s, pinned %s", e.observed, e.pinned)
}

func clientConfig(s TunnelSpec, observed *string, keyType *string) (*ssh.ClientConfig, error) {
	var methods []ssh.AuthMethod
	if s.PrivateKey != "" {
		var signer ssh.Signer
		var err error
		if s.Passphrase != "" {
			signer, err = ssh.ParsePrivateKeyWithPassphrase([]byte(s.PrivateKey), []byte(s.Passphrase))
		} else {
			signer, err = ssh.ParsePrivateKey([]byte(s.PrivateKey))
		}
		if err != nil {
			var pm *ssh.PassphraseMissingError
			if errors.As(err, &pm) {
				return nil, errors.New("the private key is encrypted: enter its passphrase")
			}
			return nil, fmt.Errorf("cannot read the private key: %v", err)
		}
		methods = append(methods, ssh.PublicKeys(signer))
	}
	if s.Password != "" {
		pw := s.Password
		methods = append(methods, ssh.Password(pw),
			ssh.KeyboardInteractive(func(_, _ string, qs []string, _ []bool) ([]string, error) {
				ans := make([]string, len(qs))
				for i := range qs {
					ans[i] = pw
				}
				return ans, nil
			}))
	}
	pinned := normFP(s.HostKeyFingerprint)
	return &ssh.ClientConfig{
		User: s.Username,
		Auth: methods,
		HostKeyCallback: func(_ string, _ net.Addr, key ssh.PublicKey) error {
			fp := ssh.FingerprintSHA256(key)
			*observed = fp
			if keyType != nil {
				*keyType = key.Type()
			}
			if pinned == "" || subtle.ConstantTimeCompare([]byte(fp), []byte(pinned)) != 1 {
				return &hostKeyError{observed: fp, pinned: pinned}
			}
			return nil
		},
		HostKeyAlgorithms: hostKeyAlgorithms,
		Timeout:           15 * time.Second,
	}, nil
}

// pipeConn turns a child process's stdin/stdout into a net.Conn (ProxyCommand transport).
type pipeConn struct {
	io.Reader
	io.WriteCloser
	cmd  *exec.Cmd
	once sync.Once
}

type pipeAddr string

func (a pipeAddr) Network() string { return "pipe" }
func (a pipeAddr) String() string  { return string(a) }

func (p *pipeConn) Close() error {
	p.once.Do(func() {
		p.WriteCloser.Close()
		if p.cmd.Process != nil {
			_ = p.cmd.Process.Kill()
		}
		_ = p.cmd.Wait()
	})
	return nil
}
func (p *pipeConn) LocalAddr() net.Addr              { return pipeAddr("cdc-sshtunnel") }
func (p *pipeConn) RemoteAddr() net.Addr             { return pipeAddr(p.cmd.Path) }
func (p *pipeConn) SetDeadline(time.Time) error      { return nil }
func (p *pipeConn) SetReadDeadline(time.Time) error  { return nil }
func (p *pipeConn) SetWriteDeadline(time.Time) error { return nil }

func (g *gateway) transport(s TunnelSpec) (net.Conn, string, error) {
	if s.Proxy.Kind == "cloudflare_access" {
		host := strings.TrimSpace(s.Proxy.Hostname)
		if host == "" {
			host = strings.TrimSpace(s.Host)
		}
		cmd := exec.Command(g.cloudflared, "access", "ssh", "--hostname", host)
		// Only what cloudflared needs: no inherited secrets, token never on argv.
		cmd.Env = []string{"HOME=/tmp", "PATH=/usr/local/bin:/usr/bin:/bin"}
		if s.Proxy.ServiceTokenID != "" {
			cmd.Env = append(cmd.Env, "TUNNEL_SERVICE_TOKEN_ID="+s.Proxy.ServiceTokenID,
				"TUNNEL_SERVICE_TOKEN_SECRET="+s.Proxy.ServiceTokenSecret)
		}
		stdin, err := cmd.StdinPipe()
		if err != nil {
			return nil, host, err
		}
		stdout, err := cmd.StdoutPipe()
		if err != nil {
			return nil, host, err
		}
		cmd.Stderr = &logWriter{prefix: "cloudflared[" + s.Name + "] "}
		if err := cmd.Start(); err != nil {
			return nil, host, fmt.Errorf("cannot start cloudflared: %v", err)
		}
		return &pipeConn{Reader: stdout, WriteCloser: stdin, cmd: cmd}, host, nil
	}
	addr := net.JoinHostPort(strings.TrimSpace(s.Host), strconv.Itoa(s.port()))
	c, err := net.DialTimeout("tcp", addr, 15*time.Second)
	return c, addr, err
}

// connect opens one authenticated SSH session. observed receives the server's host key
// fingerprint even when the handshake is refused because of it.
func (g *gateway) connect(s TunnelSpec, observed *string) (*ssh.Client, error) {
	return g.connectTyped(s, observed, nil)
}

func (g *gateway) connectTyped(s TunnelSpec, observed, keyType *string) (*ssh.Client, error) {
	cfg, err := clientConfig(s, observed, keyType)
	if err != nil {
		return nil, err
	}
	conn, addr, err := g.transport(s)
	if err != nil {
		return nil, err
	}
	// Bound the handshake: a pipe transport has no deadlines of its own.
	timer := time.AfterFunc(g.dialWait, func() { conn.Close() })
	defer timer.Stop()
	sc, chans, reqs, err := ssh.NewClientConn(conn, addr, cfg)
	if err != nil {
		conn.Close()
		return nil, err
	}
	return ssh.NewClient(sc, chans, reqs), nil
}

// run keeps one tunnel connected until ctx is cancelled, with backoff 1 s → 30 s.
func (g *gateway) run(ctx context.Context, t *tunnel) {
	backoff := time.Second
	for ctx.Err() == nil {
		g.mu.Lock()
		spec := t.spec
		g.mu.Unlock()
		var observed string
		client, err := g.connect(spec, &observed)
		g.mu.Lock()
		if observed != "" {
			t.observedFP = observed
		}
		if err != nil {
			t.lastErr, t.errAt = err.Error(), time.Now()
			g.mu.Unlock()
			log.Printf("tunnel %s: %v (retry in %s)", spec.Name, err, backoff)
			select {
			case <-ctx.Done():
				return
			case <-t.kick:
			case <-time.After(backoff):
			}
			if backoff < 30*time.Second {
				backoff *= 2
			}
			continue
		}
		if ctx.Err() != nil {
			g.mu.Unlock()
			client.Close()
			return
		}
		t.client, t.connected, t.since, t.lastErr = client, true, time.Now(), ""
		g.syncForwardsLocked(t)
		g.mu.Unlock()
		log.Printf("tunnel %s: connected (%s)", spec.Name, observed)
		backoff = time.Second

		stop := make(chan struct{})
		go keepalive(client, spec.keepalive(), stop)
		waitErr := make(chan error, 1)
		go func() { waitErr <- client.Wait() }()
		select {
		case <-ctx.Done():
			client.Close()
		case err := <-waitErr:
			g.mu.Lock()
			t.lastErr, t.errAt = fmt.Sprintf("connection lost: %v", err), time.Now()
			g.mu.Unlock()
			log.Printf("tunnel %s: connection lost: %v", spec.Name, err)
		}
		close(stop)
		client.Close()
		g.mu.Lock()
		t.client, t.connected = nil, false
		g.closeListenersLocked(t)
		g.mu.Unlock()
	}
}

func keepalive(c *ssh.Client, every time.Duration, stop chan struct{}) {
	tk := time.NewTicker(every)
	defer tk.Stop()
	for {
		select {
		case <-stop:
			return
		case <-tk.C:
			res := make(chan error, 1)
			go func() { _, _, err := c.SendRequest("keepalive@openssh.com", true, nil); res <- err }()
			select {
			case err := <-res:
				if err != nil {
					c.Close()
					return
				}
			case <-time.After(2 * every):
				c.Close()
				return
			case <-stop:
				return
			}
		}
	}
}

// ── forwards ────────────────────────────────────────────────────────────────

// syncForwardsLocked opens listeners for the spec's forwards (when connected) and closes the
// ones no longer declared. Caller holds g.mu.
func (g *gateway) syncForwardsLocked(t *tunnel) {
	want := map[int]ForwardSpec{}
	for _, f := range t.spec.Forwards {
		want[f.ListenPort] = f
	}
	for p, f := range t.forwards {
		if w, ok := want[p]; !ok || !t.connected || w != f.spec {
			g.closeForwardLocked(t, p)
		}
	}
	if !t.connected {
		return
	}
	for p, fs := range want {
		if _, ok := t.forwards[p]; ok {
			continue
		}
		ln, err := g.listen(fmt.Sprintf("0.0.0.0:%d", p))
		if err != nil {
			t.lastErr, t.errAt = fmt.Sprintf("cannot listen on :%d: %v", p, err), time.Now()
			log.Printf("tunnel %s: %s", t.spec.Name, t.lastErr)
			continue
		}
		f := &forward{spec: fs, ln: ln, conns: map[net.Conn]struct{}{}}
		t.forwards[p] = f
		go g.serve(t, f)
		log.Printf("tunnel %s: forward :%d → %s:%d", t.spec.Name, p, fs.RemoteHost, fs.RemotePort)
	}
}

func (g *gateway) closeForwardLocked(t *tunnel, port int) {
	f := t.forwards[port]
	if f == nil {
		return
	}
	f.ln.Close()
	for c := range f.conns {
		c.Close()
	}
	delete(t.forwards, port)
}

func (g *gateway) closeListenersLocked(t *tunnel) {
	for p := range t.forwards {
		g.closeForwardLocked(t, p)
	}
}

func (g *gateway) dialThrough(ctx context.Context, t *tunnel, host string, port int) (net.Conn, error) {
	g.mu.Lock()
	c := t.client
	g.mu.Unlock()
	if c == nil {
		return nil, errors.New("the SSH session is not connected")
	}
	ctx, cancel := context.WithTimeout(ctx, g.dialWait)
	defer cancel()
	return c.DialContext(ctx, "tcp", net.JoinHostPort(host, strconv.Itoa(port)))
}

func (g *gateway) serve(t *tunnel, f *forward) {
	for {
		client, err := f.ln.Accept()
		if err != nil {
			return // listener closed
		}
		go func() {
			defer client.Close()
			remote, err := g.dialThrough(context.Background(), t, f.spec.RemoteHost, f.spec.RemotePort)
			if err != nil {
				log.Printf("forward :%d → %s:%d: %v", f.spec.ListenPort, f.spec.RemoteHost, f.spec.RemotePort, err)
				return
			}
			defer remote.Close()
			g.mu.Lock()
			f.conns[client] = struct{}{}
			g.mu.Unlock()
			defer func() { g.mu.Lock(); delete(f.conns, client); g.mu.Unlock() }()
			done := make(chan struct{}, 2)
			go func() { io.Copy(remote, client); closeWrite(remote); done <- struct{}{} }()
			go func() { io.Copy(client, remote); closeWrite(client); done <- struct{}{} }()
			<-done
			<-done
		}()
	}
}

func closeWrite(c net.Conn) {
	if cw, ok := c.(interface{ CloseWrite() error }); ok {
		cw.CloseWrite()
	}
}

// ── tunnel lifecycle ────────────────────────────────────────────────────────

func (g *gateway) upsert(s TunnelSpec) error {
	if err := validateSpec(s, true); err != nil {
		return err
	}
	for _, f := range s.Forwards {
		if f.ListenPort < g.portMin || f.ListenPort > g.portMax {
			return fmt.Errorf("listen_port %d is outside %d-%d", f.ListenPort, g.portMin, g.portMax)
		}
	}
	g.mu.Lock()
	// A listen port belongs to one tunnel.
	for id, other := range g.tunnels {
		if id == s.ID {
			continue
		}
		for _, f := range s.Forwards {
			for _, o := range other.spec.Forwards {
				if o.ListenPort == f.ListenPort {
					g.mu.Unlock()
					return fmt.Errorf("listen_port %d is already used by tunnel %s", f.ListenPort, other.spec.Name)
				}
			}
		}
	}
	t, ok := g.tunnels[s.ID]
	if ok && t.spec.connKey() == s.connKey() {
		t.spec = s
		g.syncForwardsLocked(t)
		g.mu.Unlock()
		return nil
	}
	if ok {
		t.cancel()
		if t.client != nil {
			t.client.Close()
		}
		g.closeListenersLocked(t)
	}
	ctx, cancel := context.WithCancel(context.Background())
	nt := &tunnel{spec: s, forwards: map[int]*forward{}, cancel: cancel, kick: make(chan struct{}, 1)}
	g.tunnels[s.ID] = nt
	g.mu.Unlock()
	go g.run(ctx, nt)
	return nil
}

func (g *gateway) remove(id string) bool {
	g.mu.Lock()
	defer g.mu.Unlock()
	t, ok := g.tunnels[id]
	if !ok {
		return false
	}
	t.cancel()
	if t.client != nil {
		t.client.Close()
	}
	g.closeListenersLocked(t)
	delete(g.tunnels, id)
	return true
}

type forwardStatus struct {
	RemoteHost string `json:"remote_host"`
	RemotePort int    `json:"remote_port"`
	ListenPort int    `json:"listen_port"`
	Listening  bool   `json:"listening"`
	Conns      int    `json:"active_connections"`
}

type tunnelStatus struct {
	ID          string          `json:"id"`
	Name        string          `json:"name"`
	Version     string          `json:"version"`
	Connected   bool            `json:"connected"`
	Since       *time.Time      `json:"connected_since,omitempty"`
	LastError   string          `json:"last_error,omitempty"`
	ErrorAt     *time.Time      `json:"last_error_at,omitempty"`
	ObservedFP  string          `json:"observed_host_key_fingerprint,omitempty"`
	ForwardHost string          `json:"forward_host"`
	Forwards    []forwardStatus `json:"forwards"`
}

func (g *gateway) statusLocked(t *tunnel) tunnelStatus {
	st := tunnelStatus{ID: t.spec.ID, Name: t.spec.Name, Version: t.spec.Version, Connected: t.connected,
		LastError: t.lastErr, ObservedFP: t.observedFP, ForwardHost: g.publicHost, Forwards: []forwardStatus{}}
	if t.connected {
		s := t.since
		st.Since = &s
	}
	if t.lastErr != "" {
		e := t.errAt
		st.ErrorAt = &e
	}
	for _, f := range t.spec.Forwards {
		fs := forwardStatus{RemoteHost: f.RemoteHost, RemotePort: f.RemotePort, ListenPort: f.ListenPort}
		if live, ok := t.forwards[f.ListenPort]; ok {
			fs.Listening = true
			fs.Conns = len(live.conns)
		}
		st.Forwards = append(st.Forwards, fs)
	}
	return st
}

// probeSpec tests a (possibly unsaved) spec: SSH login, then optionally a TCP dial through it.
// An unpinned or mismatching host key stops before authentication, so no credential is sent to
// a server the operator has not confirmed; the observed fingerprint is returned to confirm.
func (g *gateway) probeSpec(ctx context.Context, s TunnelSpec, host string, port int) map[string]any {
	res := map[string]any{"ok": false}
	if err := validateSpec(s, false); err != nil {
		res["error"] = err.Error()
		return res
	}
	start := time.Now()
	var observed, keyType string
	client, err := g.connectTyped(s, &observed, &keyType)
	if observed != "" {
		res["host_key_fingerprint"] = observed
		res["host_key_type"] = keyType
	}
	if err != nil {
		var hk *hostKeyError
		if errors.As(err, &hk) {
			res["needs_host_key"] = hk.pinned == ""
			res["host_key_mismatch"] = hk.pinned != ""
		}
		res["error"] = err.Error()
		res["hint"] = connectHint(s, err)
		return res
	}
	defer client.Close()
	res["ssh_ok"] = true
	res["server_version"] = string(client.ServerVersion())
	res["ssh_ms"] = time.Since(start).Milliseconds()
	if host != "" && port > 0 {
		res["target"] = net.JoinHostPort(host, strconv.Itoa(port))
		t := &tunnel{client: client}
		d0 := time.Now()
		c, err := g.dialThrough(ctx, t, host, port)
		if err != nil {
			res["error"] = err.Error()
			res["hint"] = dialHint(err)
			return res
		}
		c.Close()
		res["dial_ms"] = time.Since(d0).Milliseconds()
	}
	res["ok"] = true
	return res
}

func connectHint(s TunnelSpec, err error) string {
	m := err.Error()
	switch {
	case strings.Contains(m, "host key not pinned"):
		return "Compare this fingerprint with the server's (ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub), then pin it and test again."
	case strings.Contains(m, "host key mismatch"):
		return "The server's host key changed or this is a different server. Confirm with the site before updating the pin."
	case strings.Contains(m, "unable to authenticate"):
		return "SSH login refused: check the username and that the public key is in ~/.ssh/authorized_keys (or the password)."
	case strings.Contains(m, "cloudflared"):
		return "cloudflared could not start; the gateway image must include it."
	case s.Proxy.Kind == "cloudflare_access":
		return "Cloudflare Access did not let the session through: check the Access hostname and that the service token has a Service Auth policy on the SSH application."
	case strings.Contains(m, "refused"):
		return "Nothing accepts SSH at that address and port."
	case strings.Contains(m, "no such host"):
		return "The SSH host name does not resolve from the gateway."
	}
	return "Check the SSH host, port and firewall."
}

func dialHint(err error) string {
	m := err.Error()
	switch {
	case strings.Contains(m, "prohibited") || strings.Contains(m, "administratively"):
		return "The SSH server refused the forward: it needs AllowTcpForwarding yes, and permitopen in authorized_keys must include this host:port."
	case strings.Contains(m, "refused"):
		return "SSH works but nothing listens at that address as seen from the SSH server: check the database host/port and its bind address."
	}
	return "SSH works but the database address did not answer from the SSH server: check host, port and the database firewall."
}

// ── HTTP API ────────────────────────────────────────────────────────────────

func (g *gateway) routes(token string) http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /health", func(w http.ResponseWriter, _ *http.Request) {
		writeJSON(w, 200, map[string]any{"status": "ok"})
	})
	mux.HandleFunc("GET /v1/capability", func(w http.ResponseWriter, _ *http.Request) {
		cf := ""
		if out, err := exec.Command(g.cloudflared, "--version").Output(); err == nil {
			cf = strings.TrimSpace(string(out))
		}
		writeJSON(w, 200, map[string]any{"engine": "ssh", "forward_host": g.publicHost,
			"forward_ports": fmt.Sprintf("%d-%d", g.portMin, g.portMax), "cloudflared": cf})
	})
	mux.HandleFunc("GET /v1/tunnels", func(w http.ResponseWriter, _ *http.Request) {
		g.mu.Lock()
		list := make([]tunnelStatus, 0, len(g.tunnels))
		for _, t := range g.tunnels {
			list = append(list, g.statusLocked(t))
		}
		g.mu.Unlock()
		sort.Slice(list, func(i, j int) bool { return list[i].Name < list[j].Name })
		writeJSON(w, 200, map[string]any{"tunnels": list})
	})
	mux.HandleFunc("PUT /v1/tunnels/{id}", func(w http.ResponseWriter, r *http.Request) {
		var s TunnelSpec
		if err := json.NewDecoder(io.LimitReader(r.Body, 1<<20)).Decode(&s); err != nil {
			writeErr(w, 400, "invalid JSON: "+err.Error())
			return
		}
		s.ID = r.PathValue("id")
		if err := g.upsert(s); err != nil {
			writeErr(w, 400, err.Error())
			return
		}
		g.mu.Lock()
		st := g.statusLocked(g.tunnels[s.ID])
		g.mu.Unlock()
		writeJSON(w, 200, st)
	})
	mux.HandleFunc("DELETE /v1/tunnels/{id}", func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, 200, map[string]any{"deleted": g.remove(r.PathValue("id"))})
	})
	mux.HandleFunc("POST /v1/tunnels/{id}/reconnect", func(w http.ResponseWriter, r *http.Request) {
		g.mu.Lock()
		t := g.tunnels[r.PathValue("id")]
		if t != nil {
			select {
			case t.kick <- struct{}{}:
			default:
			}
		}
		g.mu.Unlock()
		if t == nil {
			writeErr(w, 404, "tunnel not configured on the gateway")
			return
		}
		writeJSON(w, 200, map[string]any{"ok": true})
	})
	// Dial host:port through a running tunnel.
	mux.HandleFunc("POST /v1/tunnels/{id}/probe", func(w http.ResponseWriter, r *http.Request) {
		var body struct {
			Host string `json:"host"`
			Port int    `json:"port"`
		}
		if err := json.NewDecoder(io.LimitReader(r.Body, 1<<16)).Decode(&body); err != nil || body.Host == "" || body.Port <= 0 {
			writeErr(w, 400, "host and port are required")
			return
		}
		g.mu.Lock()
		t := g.tunnels[r.PathValue("id")]
		var st tunnelStatus
		if t != nil {
			st = g.statusLocked(t)
		}
		g.mu.Unlock()
		if t == nil {
			writeErr(w, 404, "tunnel not configured on the gateway")
			return
		}
		res := map[string]any{"target": net.JoinHostPort(body.Host, strconv.Itoa(body.Port)), "connected": st.Connected}
		if !st.Connected {
			res["ok"] = false
			res["error"] = "the SSH session is not connected"
			if st.LastError != "" {
				res["error"] = st.LastError
			}
			writeJSON(w, 200, res)
			return
		}
		start := time.Now()
		c, err := g.dialThrough(r.Context(), t, body.Host, body.Port)
		if err != nil {
			res["ok"] = false
			res["error"] = err.Error()
			res["hint"] = dialHint(err)
		} else {
			c.Close()
			res["ok"] = true
			res["ms"] = time.Since(start).Milliseconds()
		}
		writeJSON(w, 200, res)
	})
	// One-shot test of a spec that may not be saved yet.
	mux.HandleFunc("POST /v1/probe", func(w http.ResponseWriter, r *http.Request) {
		var body struct {
			Tunnel TunnelSpec `json:"tunnel"`
			Host   string     `json:"host"`
			Port   int        `json:"port"`
		}
		if err := json.NewDecoder(io.LimitReader(r.Body, 1<<20)).Decode(&body); err != nil {
			writeErr(w, 400, "invalid JSON: "+err.Error())
			return
		}
		if body.Tunnel.ID == "" {
			body.Tunnel.ID = "probe"
		}
		writeJSON(w, 200, g.probeSpec(r.Context(), body.Tunnel, body.Host, body.Port))
	})
	return auth(token, mux)
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

type logWriter struct{ prefix string }

func (l *logWriter) Write(p []byte) (int, error) {
	for _, line := range strings.Split(strings.TrimRight(string(p), "\n"), "\n") {
		if line != "" {
			log.Print(l.prefix + line)
		}
	}
	return len(p), nil
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
	addr := fmt.Sprintf("%s:%d", envStr("SSHT_API_HOST", "0.0.0.0"), envInt("SSHT_API_PORT", 8092))
	token := envStr("SSHTUNNEL_API_TOKEN", "")
	if token == "" {
		log.Printf("SSHTUNNEL_API_TOKEN is not set: the API accepts any caller on the Docker network")
	}
	log.Printf("cdc-sshtunnel listening on %s (forwards %d-%d as %s)", addr, g.portMin, g.portMax, g.publicHost)
	log.Fatal(http.ListenAndServe(addr, g.routes(token)))
}
