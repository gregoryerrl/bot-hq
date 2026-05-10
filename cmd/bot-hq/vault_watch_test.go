// Phase T T-10 cycle-3: vault_watch tests. parseFileScheme + startVaultWatcher
// integration coverage. R39 TEST-ISOLATION via t.TempDir() + t.Setenv("HOME", ...)
// to avoid the live-filesystem ~ expansion path.

package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func TestParseFileScheme(t *testing.T) {
	cases := []struct {
		name     string
		ref      string
		wantPath string
		wantKey  string
		wantErr  bool
	}{
		{
			name:     "absolute_path",
			ref:      "file:/tmp/vault.env#DEEPSEEK_API_KEY",
			wantPath: "/tmp/vault.env",
			wantKey:  "DEEPSEEK_API_KEY",
		},
		{
			name:    "missing_hash_suffix",
			ref:     "file:/tmp/vault.env",
			wantErr: true,
		},
		{
			name:    "empty_path",
			ref:     "file:#KEY",
			wantErr: true,
		},
		{
			name:    "empty_key",
			ref:     "file:/tmp/vault.env#",
			wantErr: true,
		},
		{
			name:    "wrong_scheme",
			ref:     "env:DEEPSEEK_API_KEY",
			wantErr: true,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			path, key, err := parseFileScheme(tc.ref)
			if tc.wantErr {
				if err == nil {
					t.Fatalf("expected error; got path=%q key=%q", path, key)
				}
				return
			}
			if err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			if path != tc.wantPath {
				t.Errorf("path = %q, want %q", path, tc.wantPath)
			}
			if key != tc.wantKey {
				t.Errorf("key = %q, want %q", key, tc.wantKey)
			}
		})
	}
}

func TestParseFileScheme_HomeExpansion(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("HOME", tmp)

	gotPath, _, err := parseFileScheme("file:~/sub/vault.env#KEY")
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	wantPath := filepath.Join(tmp, "sub", "vault.env")
	if gotPath != wantPath {
		t.Errorf("expanded path = %q, want %q", gotPath, wantPath)
	}
}

// TestStartVaultWatcher_DiscoversFileSchemeAndFiresOnChange validates the
// full daemon-side wire-up: rows with file: scheme are picked up, the
// watcher fires on mtime advance, and a hub MsgUpdate with vault-watcher
// body is inserted.
func TestStartVaultWatcher_DiscoversFileSchemeAndFiresOnChange(t *testing.T) {
	dir := t.TempDir()
	vaultPath := filepath.Join(dir, "rain.env")
	if err := os.WriteFile(vaultPath, []byte("DEEPSEEK_API_KEY=v1\n"), 0o600); err != nil {
		t.Fatalf("write vault: %v", err)
	}

	// Build a hub with isolated DB.
	cfg := hub.Config{}
	cfg.Hub.DBPath = filepath.Join(dir, "hub.db")
	h, err := hub.NewHub(cfg)
	if err != nil {
		t.Fatalf("NewHub: %v", err)
	}
	if err := h.Start(); err != nil {
		t.Fatalf("Start: %v", err)
	}
	t.Cleanup(func() { _ = h.Stop() })

	// Override rain row to file: scheme pointing at the test vault.
	override := &hub.AgentModelConfig{
		AgentID:       "rain",
		Provider:      "deepseek",
		ModelName:     "deepseek-v4-pro",
		BaseURL:       "https://api.deepseek.com/anthropic",
		AuthSecretRef: "file:" + vaultPath + "#DEEPSEEK_API_KEY",
		Enabled:       true,
		Notes:         "T-10 watcher test override",
	}
	if err := h.DB.SetAgentModelConfig(override); err != nil {
		t.Fatalf("Set override: %v", err)
	}

	w := startVaultWatcher(h)
	if w == nil {
		t.Fatal("expected watcher; got nil")
	}
	t.Cleanup(w.Stop)

	// Snapshot pre-edit message-id high-watermark.
	preIDs, err := h.DB.ReadMessages("", 0, 1000)
	if err != nil {
		t.Fatalf("ReadMessages pre: %v", err)
	}
	preMax := int64(0)
	for _, m := range preIDs {
		if m.ID > preMax {
			preMax = m.ID
		}
	}

	// Bump mtime to simulate a rotation.
	future := time.Now().Add(2 * time.Second)
	if err := os.Chtimes(vaultPath, future, future); err != nil {
		t.Fatalf("chtimes: %v", err)
	}

	// Force a synchronous check (production path uses ticker; tests would
	// otherwise wait the full poll interval).
	w.CheckOnce()

	// Verify a system MsgUpdate with vault-watcher body landed.
	postIDs, err := h.DB.ReadMessages("", preMax, 1000)
	if err != nil {
		t.Fatalf("ReadMessages post: %v", err)
	}
	found := false
	for _, m := range postIDs {
		if m.FromAgent != "system" || m.Type != protocol.MsgUpdate {
			continue
		}
		if strings.Contains(m.Content, "vault-watcher") &&
			strings.Contains(m.Content, "rotation-detected") &&
			strings.Contains(m.Content, "agent=rain") {
			found = true
			break
		}
	}
	if !found {
		t.Errorf("did not find vault-watcher rotation-detected message; got %d post messages", len(postIDs))
	}
}
