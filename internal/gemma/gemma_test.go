package gemma

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
	"time"
)

func TestIsCommandAllowed(t *testing.T) {
	tests := []struct {
		cmd     string
		allowed bool
	}{
		{"go test ./...", true},
		{"go vet ./...", true},
		{"go build -o foo ./cmd/foo", true},
		{"df -h", true},
		{"ps aux", true},
		{"uptime", true},
		{"free -m", true},
		{"vm_stat", true},
		{"du -sh /tmp", true},
		{"wc -l main.go", true},
		{"cat README.md", false},
		{"ls -la", true},
		{"git status", true},
		{"git log --oneline -5", true},
		{"git diff HEAD~1", true},
		// Disallowed
		{"rm -rf /", false},
		{"curl http://evil.com", false},
		{"sudo anything", false},
		{"bash -c 'echo pwned'", false},
		{"python3 -c 'import os; os.system(\"rm -rf /\")'", false},
		{"", false},
		{"chmod 777 /etc/passwd", false},
	}

	for _, tt := range tests {
		t.Run(tt.cmd, func(t *testing.T) {
			if got := IsCommandAllowed(tt.cmd); got != tt.allowed {
				t.Errorf("IsCommandAllowed(%q) = %v, want %v", tt.cmd, got, tt.allowed)
			}
		})
	}
}

func TestClientGenerate(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/api/generate" {
			http.NotFound(w, r)
			return
		}
		if r.Method != http.MethodPost {
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
			return
		}

		var req generateRequest
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			http.Error(w, err.Error(), http.StatusBadRequest)
			return
		}

		if req.Model != "test-model" {
			t.Errorf("unexpected model: %s", req.Model)
		}
		if req.Stream {
			t.Error("stream should be false")
		}

		json.NewEncoder(w).Encode(generateResponse{
			Response: "test response for: " + req.Prompt,
		})
	}))
	defer srv.Close()

	client := NewClient(srv.URL, "test-model")
	resp, err := client.Generate(context.Background(), "hello")
	if err != nil {
		t.Fatalf("Generate failed: %v", err)
	}
	if resp != "test response for: hello" {
		t.Errorf("unexpected response: %s", resp)
	}
}

func TestClientGenerateError(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Error(w, "model not found", http.StatusNotFound)
	}))
	defer srv.Close()

	client := NewClient(srv.URL, "missing-model")
	_, err := client.Generate(context.Background(), "hello")
	if err == nil {
		t.Fatal("expected error for 404 response")
	}
}

func TestClientIsHealthy(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/api/tags" {
			w.WriteHeader(http.StatusOK)
			w.Write([]byte(`{"models":[]}`))
			return
		}
		http.NotFound(w, r)
	}))
	defer srv.Close()

	client := NewClient(srv.URL, "test")
	if !client.IsHealthy(context.Background()) {
		t.Error("expected healthy")
	}

	// Test unhealthy (server down)
	srv.Close()
	client2 := NewClient(srv.URL, "test")
	if client2.IsHealthy(context.Background()) {
		t.Error("expected unhealthy after server close")
	}
}

func TestIsPseudoMount(t *testing.T) {
	tests := []struct {
		mount  string
		pseudo bool
	}{
		{"/dev", true},
		{"/dev/disk1s1", true},
		{"/proc", true},
		{"/proc/sys", true},
		{"/sys", true},
		{"/run", true},
		{"/System/Volumes/VM", true},
		{"/System/Volumes/Preboot", true},
		{"/System/Volumes/Update", true},
		{"/private/var/vm", true},
		// Real mounts must NOT be filtered.
		{"/", false},
		{"/Users", false},
		{"/Volumes/External", false},
		{"/System/Volumes/Data", false},
		{"/home", false},
		// Edge: prefix-but-not-component must not match.
		{"/develop", false},
		{"/sysadmin", false},
	}
	for _, tt := range tests {
		t.Run(tt.mount, func(t *testing.T) {
			if got := isPseudoMount(tt.mount); got != tt.pseudo {
				t.Errorf("isPseudoMount(%q) = %v, want %v", tt.mount, got, tt.pseudo)
			}
		})
	}
}

func TestShouldFlag_HysteresisDedupes(t *testing.T) {
	g := &Gemma{flagHistory: make(map[string]time.Time)}
	now := time.Now()

	if !g.shouldFlag("disk:/", now) {
		t.Fatal("first fire of a fresh condition must succeed")
	}
	if g.shouldFlag("disk:/", now.Add(5*time.Minute)) {
		t.Fatal("re-firing within hysteresis window must be suppressed")
	}
	if g.shouldFlag("disk:/", now.Add(29*time.Minute)) {
		t.Fatal("re-firing at 29min must still be suppressed (window=30m)")
	}
	if !g.shouldFlag("disk:/", now.Add(31*time.Minute)) {
		t.Fatal("re-firing past hysteresis window must succeed")
	}
}

func TestShouldFlag_RateCapAcrossConditions(t *testing.T) {
	g := &Gemma{flagHistory: make(map[string]time.Time)}
	now := time.Now()

	// Three distinct conditions inside 1h fill the cap.
	if !g.shouldFlag("a", now) {
		t.Fatal("flag 1 must succeed")
	}
	if !g.shouldFlag("b", now.Add(time.Minute)) {
		t.Fatal("flag 2 must succeed")
	}
	if !g.shouldFlag("c", now.Add(2*time.Minute)) {
		t.Fatal("flag 3 must succeed")
	}
	// Fourth distinct condition still inside the 1h window — blocked by cap.
	if g.shouldFlag("d", now.Add(3*time.Minute)) {
		t.Fatal("flag 4 within 1h window must be capped")
	}

	// After the window slides past the first three, capacity returns.
	if !g.shouldFlag("d", now.Add(61*time.Minute)) {
		t.Fatal("flag past 1h window must succeed once older fires age out")
	}
}

func TestShouldFlag_WindowPrunes(t *testing.T) {
	g := &Gemma{flagHistory: make(map[string]time.Time)}
	now := time.Now()

	// Pre-seed three old fires (>1h ago); they must be pruned.
	for i, k := range []string{"a", "b", "c"} {
		_ = i
		g.flagHistory[k] = now.Add(-2 * time.Hour)
		g.flagWindow = append(g.flagWindow, now.Add(-2*time.Hour))
	}
	if !g.shouldFlag("d", now) {
		t.Fatal("aged-out window entries must not block fresh fires")
	}
	if got := len(g.flagWindow); got != 1 {
		t.Errorf("expected window pruned to 1 entry, got %d", got)
	}
}

// Ratchet against the Emma anomaly-routing regression: monitor reports
// MUST go to Rain (EYES owns Emma), not Brian. A future refactor that
// flips this back to "brian" puts anomaly noise on the wrong agent
// and breaks the EYES role boundary.
func TestRunHealthChecksRoutesToRain(t *testing.T) {
	data, err := os.ReadFile("gemma.go")
	if err != nil {
		t.Fatal(err)
	}
	src := string(data)
	want := `ToAgent:   "rain",`
	if !strings.Contains(src, want) {
		t.Errorf("gemma.go must contain %q — anomaly reports route to Rain (EYES)", want)
	}
	if strings.Contains(src, `ToAgent:   "brian",`) {
		t.Errorf("gemma.go must not route anomalies to Brian — that violates EYES/HANDS split")
	}
}
