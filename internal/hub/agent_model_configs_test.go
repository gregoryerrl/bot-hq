package hub

import (
	"path/filepath"
	"testing"
)

func newTestDBForAgentConfigs(t *testing.T) *DB {
	t.Helper()
	dir := t.TempDir()
	db, err := OpenDB(filepath.Join(dir, "hub.db"))
	if err != nil {
		t.Fatalf("OpenDB: %v", err)
	}
	t.Cleanup(func() { _ = db.Close() })
	return db
}

func TestAgentModelConfigs_seedDefaults(t *testing.T) {
	db := newTestDBForAgentConfigs(t)

	// migrate already ran during OpenDB; verify defaults seeded
	configs, err := db.ListAgentModelConfigs(false)
	if err != nil {
		t.Fatalf("ListAgentModelConfigs: %v", err)
	}

	// Should have 5 default rows: brian / rain / emma / clive / coder-template
	want := 5
	if len(configs) != want {
		t.Errorf("default seed count = %d, want %d", len(configs), want)
	}

	// Verify each default-seed agent_id is present
	seen := make(map[string]bool)
	for _, c := range configs {
		seen[c.AgentID] = true
	}
	for _, defID := range []string{"brian", "rain", "emma", "clive", "coder-template"} {
		if !seen[defID] {
			t.Errorf("default-seed missing for %s", defID)
		}
	}
}

func TestAgentModelConfigs_brianDefaults(t *testing.T) {
	db := newTestDBForAgentConfigs(t)
	cfg, err := db.GetAgentModelConfig("brian")
	if err != nil {
		t.Fatalf("Get brian: %v", err)
	}
	if cfg.Provider != "anthropic" {
		t.Errorf("brian provider = %q, want anthropic", cfg.Provider)
	}
	if cfg.AuthSecretRef != "oauth:CLAUDE_CODE_OAUTH_TOKEN" {
		t.Errorf("brian auth_secret_ref = %q, want oauth:CLAUDE_CODE_OAUTH_TOKEN", cfg.AuthSecretRef)
	}
	if !cfg.Enabled {
		t.Error("brian should be enabled by default")
	}
	if cfg.BaseURL != "" {
		t.Errorf("brian base_url = %q, want empty (provider default)", cfg.BaseURL)
	}
}

func TestAgentModelConfigs_rainDeepSeek(t *testing.T) {
	db := newTestDBForAgentConfigs(t)
	cfg, err := db.GetAgentModelConfig("rain")
	if err != nil {
		t.Fatalf("Get rain: %v", err)
	}
	if cfg.Provider != "deepseek" {
		t.Errorf("rain provider = %q, want deepseek", cfg.Provider)
	}
	if cfg.ModelName != "deepseek-v4-pro" {
		t.Errorf("rain model_name = %q, want deepseek-v4-pro", cfg.ModelName)
	}
	if cfg.BaseURL != "https://api.deepseek.com/anthropic" {
		t.Errorf("rain base_url = %q, want Anthropic-compatible endpoint", cfg.BaseURL)
	}
	if cfg.AuthSecretRef != "env:DEEPSEEK_API_KEY" {
		t.Errorf("rain auth_secret_ref = %q, want env:DEEPSEEK_API_KEY (reference-pointer per R52)", cfg.AuthSecretRef)
	}
}

func TestAgentModelConfigs_setUpdate(t *testing.T) {
	db := newTestDBForAgentConfigs(t)

	// Update rain notes
	cfg, err := db.GetAgentModelConfig("rain")
	if err != nil {
		t.Fatalf("Get: %v", err)
	}
	originalCreatedAt := cfg.CreatedAt
	cfg.Notes = "updated notes"
	if err := db.SetAgentModelConfig(cfg); err != nil {
		t.Fatalf("Set: %v", err)
	}

	// Re-fetch and verify
	updated, err := db.GetAgentModelConfig("rain")
	if err != nil {
		t.Fatalf("Re-Get: %v", err)
	}
	if updated.Notes != "updated notes" {
		t.Errorf("notes = %q, want updated", updated.Notes)
	}
	if !updated.CreatedAt.Equal(originalCreatedAt) {
		t.Errorf("CreatedAt mutated on update: original=%v updated=%v", originalCreatedAt, updated.CreatedAt)
	}
	if !updated.UpdatedAt.After(originalCreatedAt) && !updated.UpdatedAt.Equal(originalCreatedAt) {
		t.Errorf("UpdatedAt should be >= CreatedAt: created=%v updated=%v", originalCreatedAt, updated.UpdatedAt)
	}
}

func TestAgentModelConfigs_setNewCustom(t *testing.T) {
	db := newTestDBForAgentConfigs(t)

	custom := &AgentModelConfig{
		AgentID:       "project:bcc-ad-manager:rain",
		Provider:      "deepseek",
		ModelName:     "deepseek-v4-pro",
		BaseURL:       "https://api.deepseek.com/anthropic",
		AuthSecretRef: "env:DEEPSEEK_API_KEY_BCC",
		Enabled:       true,
		Notes:         "Per-project rain override for bcc-ad-manager (R52 multi-project extensibility)",
	}
	if err := db.SetAgentModelConfig(custom); err != nil {
		t.Fatalf("Set custom: %v", err)
	}

	got, err := db.GetAgentModelConfig("project:bcc-ad-manager:rain")
	if err != nil {
		t.Fatalf("Get custom: %v", err)
	}
	if got.AuthSecretRef != "env:DEEPSEEK_API_KEY_BCC" {
		t.Errorf("custom auth_secret_ref round-trip: got %q", got.AuthSecretRef)
	}
}

func TestAgentModelConfigs_listEnabledOnly(t *testing.T) {
	db := newTestDBForAgentConfigs(t)

	// Disable rain
	cfg, err := db.GetAgentModelConfig("rain")
	if err != nil {
		t.Fatalf("Get: %v", err)
	}
	cfg.Enabled = false
	if err := db.SetAgentModelConfig(cfg); err != nil {
		t.Fatalf("Set: %v", err)
	}

	// List enabled-only should exclude rain
	enabled, err := db.ListAgentModelConfigs(true)
	if err != nil {
		t.Fatalf("List enabled: %v", err)
	}
	for _, c := range enabled {
		if c.AgentID == "rain" {
			t.Error("rain should be excluded from enabled-only list")
		}
	}

	// List all should include rain
	all, err := db.ListAgentModelConfigs(false)
	if err != nil {
		t.Fatalf("List all: %v", err)
	}
	rainFound := false
	for _, c := range all {
		if c.AgentID == "rain" {
			rainFound = true
		}
	}
	if !rainFound {
		t.Error("rain missing from all-list")
	}
}

func TestAgentModelConfigs_delete(t *testing.T) {
	db := newTestDBForAgentConfigs(t)

	// Add a custom row
	custom := &AgentModelConfig{
		AgentID:       "test-delete-target",
		Provider:      "anthropic",
		ModelName:     "claude-default",
		AuthSecretRef: "oauth:CLAUDE_CODE_OAUTH_TOKEN",
		Enabled:       true,
	}
	if err := db.SetAgentModelConfig(custom); err != nil {
		t.Fatalf("Set: %v", err)
	}

	// Delete it
	if err := db.DeleteAgentModelConfig("test-delete-target"); err != nil {
		t.Fatalf("Delete: %v", err)
	}

	// Verify gone
	_, err := db.GetAgentModelConfig("test-delete-target")
	if err != ErrAgentModelConfigNotFound {
		t.Errorf("after delete: err = %v, want ErrAgentModelConfigNotFound", err)
	}

	// Idempotent: double-delete should not error
	if err := db.DeleteAgentModelConfig("test-delete-target"); err != nil {
		t.Errorf("double-delete should be idempotent: %v", err)
	}
}

func TestAgentModelConfigs_setValidation(t *testing.T) {
	db := newTestDBForAgentConfigs(t)

	cases := []struct {
		name string
		cfg  *AgentModelConfig
	}{
		{"empty agent_id", &AgentModelConfig{Provider: "anthropic", ModelName: "x", AuthSecretRef: "env:X"}},
		{"empty provider", &AgentModelConfig{AgentID: "x", ModelName: "x", AuthSecretRef: "env:X"}},
		{"empty model", &AgentModelConfig{AgentID: "x", Provider: "anthropic", AuthSecretRef: "env:X"}},
		{"empty secret_ref", &AgentModelConfig{AgentID: "x", Provider: "anthropic", ModelName: "x"}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if err := db.SetAgentModelConfig(tc.cfg); err == nil {
				t.Errorf("expected validation error for %s", tc.name)
			}
		})
	}
}

func TestAgentModelConfigs_isDefaultSeed(t *testing.T) {
	cases := []struct {
		agentID string
		want    bool
	}{
		{"brian", true},
		{"rain", true},
		{"emma", true},
		{"clive", true},
		{"coder-template", true},
		{"project:bcc-ad-manager:rain", false},
		{"random-custom-agent", false},
	}
	for _, tc := range cases {
		t.Run(tc.agentID, func(t *testing.T) {
			got := IsDefaultSeed(tc.agentID)
			if got != tc.want {
				t.Errorf("IsDefaultSeed(%q) = %v, want %v", tc.agentID, got, tc.want)
			}
		})
	}
}

func TestAgentModelConfigs_migrationIdempotent(t *testing.T) {
	db := newTestDBForAgentConfigs(t)

	// First migrate ran during OpenDB; explicitly re-run
	if err := db.migrateAgentModelConfigs(); err != nil {
		t.Fatalf("re-migrate: %v", err)
	}

	// Verify defaults still 5 (not duplicated)
	configs, _ := db.ListAgentModelConfigs(false)
	if len(configs) != 5 {
		t.Errorf("after re-migrate, count = %d, want 5 (idempotent)", len(configs))
	}
}
