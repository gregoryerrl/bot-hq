package hub

import (
	"os"
	"path/filepath"
	"testing"
	"time"
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

func TestAgentModelConfigs_emmaDeepSeek(t *testing.T) {
	db := newTestDBForAgentConfigs(t)
	cfg, err := db.GetAgentModelConfig("emma")
	if err != nil {
		t.Fatalf("Get emma: %v", err)
	}
	if cfg.Provider != "deepseek" {
		t.Errorf("emma provider = %q, want deepseek (Z-9d per vision.md)", cfg.Provider)
	}
	if cfg.ModelName != "deepseek-v4-pro" {
		t.Errorf("emma model_name = %q, want deepseek-v4-pro", cfg.ModelName)
	}
	if cfg.BaseURL != "https://api.deepseek.com/anthropic" {
		t.Errorf("emma base_url = %q, want Anthropic-compatible endpoint", cfg.BaseURL)
	}
	if cfg.AuthSecretRef != "file:~/.bot-hq/agents/rain/.env#DEEPSEEK_API_KEY" {
		t.Errorf("emma auth_secret_ref = %q, want shared rain vault path (Z-9d)", cfg.AuthSecretRef)
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
	if cfg.AuthSecretRef != "file:~/.bot-hq/agents/rain/.env#DEEPSEEK_API_KEY" {
		t.Errorf("rain auth_secret_ref = %q, want file:~/.bot-hq/agents/rain/.env#DEEPSEEK_API_KEY (FileVault per T-8.10 + T-9 cycle-3 default-alignment)", cfg.AuthSecretRef)
	}
}

// TestMigrateEnvSchemeToFileVaultForRain validates the cycle-3 T-9 idempotent
// migration: existing rain rows seeded under cycle-2 default `env:DEEPSEEK_API_KEY`
// are auto-converted to `file:~/.bot-hq/agents/rain/.env#DEEPSEEK_API_KEY` when
// the vault file is present. R39 TEST-ISOLATION via t.TempDir() vault path.
func TestMigrateEnvSchemeToFileVaultForRain(t *testing.T) {
	cases := []struct {
		name        string
		startScheme string
		writeVault  bool
		wantScheme  string
	}{
		{
			name:        "env_scheme_with_vault_present_migrates",
			startScheme: "env:DEEPSEEK_API_KEY",
			writeVault:  true,
			wantScheme:  "file:~/.bot-hq/agents/rain/.env#DEEPSEEK_API_KEY",
		},
		{
			name:        "env_scheme_without_vault_left_alone",
			startScheme: "env:DEEPSEEK_API_KEY",
			writeVault:  false,
			wantScheme:  "env:DEEPSEEK_API_KEY",
		},
		{
			name:        "file_scheme_idempotent_noop",
			startScheme: "file:~/.bot-hq/agents/rain/.env#DEEPSEEK_API_KEY",
			writeVault:  true,
			wantScheme:  "file:~/.bot-hq/agents/rain/.env#DEEPSEEK_API_KEY",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			db := newTestDBForAgentConfigs(t)

			// Override default seeded scheme for this case.
			cfg, err := db.GetAgentModelConfig("rain")
			if err != nil {
				t.Fatalf("Get rain: %v", err)
			}
			cfg.AuthSecretRef = tc.startScheme
			if err := db.SetAgentModelConfig(cfg); err != nil {
				t.Fatalf("Set rain: %v", err)
			}

			// Configure isolated vault path; conditionally create vault file.
			vaultDir := t.TempDir()
			vaultFile := filepath.Join(vaultDir, "rain.env")
			if tc.writeVault {
				if err := os.WriteFile(vaultFile, []byte("DEEPSEEK_API_KEY=sk-test\n"), 0600); err != nil {
					t.Fatalf("write vault: %v", err)
				}
			}
			old := vaultPathFnForRain
			vaultPathFnForRain = func() (string, error) { return vaultFile, nil }
			t.Cleanup(func() { vaultPathFnForRain = old })

			if err := db.migrateEnvSchemeToFileVaultForRain(); err != nil {
				t.Fatalf("migrate: %v", err)
			}

			got, err := db.GetAgentModelConfig("rain")
			if err != nil {
				t.Fatalf("Get post-migrate: %v", err)
			}
			if got.AuthSecretRef != tc.wantScheme {
				t.Errorf("AuthSecretRef = %q, want %q", got.AuthSecretRef, tc.wantScheme)
			}
		})
	}
}

// TestMigrateEnvSchemeToFileVaultForRain_idempotentRunTwice validates that
// running the migration twice is safe — the first run converts env: → file:,
// the second run is a no-op (no double-update, UpdatedAt should not advance
// on a no-op run). Per R37/R39 discipline.
func TestMigrateEnvSchemeToFileVaultForRain_idempotentRunTwice(t *testing.T) {
	db := newTestDBForAgentConfigs(t)

	// Seed env: scheme on rain row to simulate cycle-2-installed DB.
	cfg, err := db.GetAgentModelConfig("rain")
	if err != nil {
		t.Fatalf("Get rain: %v", err)
	}
	cfg.AuthSecretRef = "env:DEEPSEEK_API_KEY"
	if err := db.SetAgentModelConfig(cfg); err != nil {
		t.Fatalf("Set rain: %v", err)
	}

	// Configure isolated vault path with vault file present.
	vaultDir := t.TempDir()
	vaultFile := filepath.Join(vaultDir, "rain.env")
	if err := os.WriteFile(vaultFile, []byte("DEEPSEEK_API_KEY=sk-test\n"), 0600); err != nil {
		t.Fatalf("write vault: %v", err)
	}
	old := vaultPathFnForRain
	vaultPathFnForRain = func() (string, error) { return vaultFile, nil }
	t.Cleanup(func() { vaultPathFnForRain = old })

	// First run: env: → file:
	if err := db.migrateEnvSchemeToFileVaultForRain(); err != nil {
		t.Fatalf("first migrate: %v", err)
	}
	afterFirst, err := db.GetAgentModelConfig("rain")
	if err != nil {
		t.Fatalf("Get post-first: %v", err)
	}
	if afterFirst.AuthSecretRef != "file:~/.bot-hq/agents/rain/.env#DEEPSEEK_API_KEY" {
		t.Fatalf("first run AuthSecretRef = %q, want file:scheme", afterFirst.AuthSecretRef)
	}
	firstUpdatedAt := afterFirst.UpdatedAt

	// Sleep briefly to ensure UpdatedAt would change if a write occurred.
	time.Sleep(time.Second)

	// Second run: no-op (already file: scheme)
	if err := db.migrateEnvSchemeToFileVaultForRain(); err != nil {
		t.Fatalf("second migrate: %v", err)
	}
	afterSecond, err := db.GetAgentModelConfig("rain")
	if err != nil {
		t.Fatalf("Get post-second: %v", err)
	}
	if afterSecond.AuthSecretRef != "file:~/.bot-hq/agents/rain/.env#DEEPSEEK_API_KEY" {
		t.Errorf("second run AuthSecretRef changed = %q, want unchanged file:scheme", afterSecond.AuthSecretRef)
	}
	if !afterSecond.UpdatedAt.Equal(firstUpdatedAt) {
		t.Errorf("second run mutated UpdatedAt = %v, want unchanged %v (no-op should not write)", afterSecond.UpdatedAt, firstUpdatedAt)
	}
}

// TestMigrateEmmaHaikuToDeepSeek validates the Z-9d idempotent migration:
// existing emma rows seeded under Phase T cycle-3 default (claude-haiku-default
// + oauth:CLAUDE_CODE_OAUTH_TOKEN) are auto-converted to DeepSeek-V4-Pro
// sharing rain's vault path when that vault file exists. User overrides (any
// field differing from the pre-Z-9d default) are left untouched.
func TestMigrateEmmaHaikuToDeepSeek(t *testing.T) {
	cases := []struct {
		name        string
		startCfg    AgentModelConfig // overrides emma row before migration
		writeVault  bool
		wantPost    AgentModelConfig // expected post-migration shape (subset of fields checked)
	}{
		{
			name: "haiku_default_with_vault_migrates",
			startCfg: AgentModelConfig{
				Provider:      "anthropic",
				ModelName:     "claude-haiku-default",
				BaseURL:       "",
				AuthSecretRef: "oauth:CLAUDE_CODE_OAUTH_TOKEN",
			},
			writeVault: true,
			wantPost: AgentModelConfig{
				Provider:      "deepseek",
				ModelName:     "deepseek-v4-pro",
				BaseURL:       "https://api.deepseek.com/anthropic",
				AuthSecretRef: "file:~/.bot-hq/agents/rain/.env#DEEPSEEK_API_KEY",
			},
		},
		{
			name: "haiku_default_without_vault_left_alone",
			startCfg: AgentModelConfig{
				Provider:      "anthropic",
				ModelName:     "claude-haiku-default",
				BaseURL:       "",
				AuthSecretRef: "oauth:CLAUDE_CODE_OAUTH_TOKEN",
			},
			writeVault: false,
			wantPost: AgentModelConfig{
				Provider:      "anthropic",
				ModelName:     "claude-haiku-default",
				BaseURL:       "",
				AuthSecretRef: "oauth:CLAUDE_CODE_OAUTH_TOKEN",
			},
		},
		{
			name: "already_deepseek_idempotent_noop",
			startCfg: AgentModelConfig{
				Provider:      "deepseek",
				ModelName:     "deepseek-v4-pro",
				BaseURL:       "https://api.deepseek.com/anthropic",
				AuthSecretRef: "file:~/.bot-hq/agents/rain/.env#DEEPSEEK_API_KEY",
			},
			writeVault: true,
			wantPost: AgentModelConfig{
				Provider:      "deepseek",
				ModelName:     "deepseek-v4-pro",
				BaseURL:       "https://api.deepseek.com/anthropic",
				AuthSecretRef: "file:~/.bot-hq/agents/rain/.env#DEEPSEEK_API_KEY",
			},
		},
		{
			name: "user_override_haiku_with_custom_secret_left_alone",
			startCfg: AgentModelConfig{
				Provider:      "anthropic",
				ModelName:     "claude-haiku-default",
				BaseURL:       "",
				AuthSecretRef: "env:CUSTOM_TOKEN", // user override
			},
			writeVault: true,
			wantPost: AgentModelConfig{
				Provider:      "anthropic",
				ModelName:     "claude-haiku-default",
				BaseURL:       "",
				AuthSecretRef: "env:CUSTOM_TOKEN",
			},
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			db := newTestDBForAgentConfigs(t)

			// Reset emma row to the case-specific starting shape.
			cfg, err := db.GetAgentModelConfig("emma")
			if err != nil {
				t.Fatalf("Get emma: %v", err)
			}
			cfg.Provider = tc.startCfg.Provider
			cfg.ModelName = tc.startCfg.ModelName
			cfg.BaseURL = tc.startCfg.BaseURL
			cfg.AuthSecretRef = tc.startCfg.AuthSecretRef
			if err := db.SetAgentModelConfig(cfg); err != nil {
				t.Fatalf("Set emma: %v", err)
			}

			vaultDir := t.TempDir()
			vaultFile := filepath.Join(vaultDir, "rain.env")
			if tc.writeVault {
				if err := os.WriteFile(vaultFile, []byte("DEEPSEEK_API_KEY=sk-test\n"), 0600); err != nil {
					t.Fatalf("write vault: %v", err)
				}
			}
			old := vaultPathFnForRain
			vaultPathFnForRain = func() (string, error) { return vaultFile, nil }
			t.Cleanup(func() { vaultPathFnForRain = old })

			if err := db.migrateEmmaHaikuToDeepSeek(); err != nil {
				t.Fatalf("migrate: %v", err)
			}

			got, err := db.GetAgentModelConfig("emma")
			if err != nil {
				t.Fatalf("Get post-migrate: %v", err)
			}
			if got.Provider != tc.wantPost.Provider {
				t.Errorf("Provider = %q, want %q", got.Provider, tc.wantPost.Provider)
			}
			if got.ModelName != tc.wantPost.ModelName {
				t.Errorf("ModelName = %q, want %q", got.ModelName, tc.wantPost.ModelName)
			}
			if got.BaseURL != tc.wantPost.BaseURL {
				t.Errorf("BaseURL = %q, want %q", got.BaseURL, tc.wantPost.BaseURL)
			}
			if got.AuthSecretRef != tc.wantPost.AuthSecretRef {
				t.Errorf("AuthSecretRef = %q, want %q", got.AuthSecretRef, tc.wantPost.AuthSecretRef)
			}
		})
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
