package gemma

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
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
