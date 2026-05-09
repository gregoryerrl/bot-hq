// Per-agent model-config infrastructure (Phase T-1; R51 PER-AGENT-MODEL-CONFIG-DISCIPLINE
// + R52 HUB-DB-CONFIG-DISCIPLINE per phase-t.md v5).
//
// hub.db `agent_model_configs` table is the single-source-of-truth for per-agent
// model-configuration. The bot-hq daemon resolves the row at agent-spawn-time and
// injects the secret-reference resolution as ANTHROPIC_BASE_URL + ANTHROPIC_AUTH_TOKEN
// + ANTHROPIC_MODEL env-var swap to the claude CLI subprocess (R43 narrowed:
// provider-agnostic subprocess pattern).
//
// Reference-pointer secret-storage strategy: hub.db stores `auth_secret_ref` as
// a reference-pointer (`oauth:<VAR>` / `env:<VAR>` / `keychain:<ID>` /
// `file:<PATH>#<KEY>`); the actual secret resolves at agent-spawn-time from the
// referenced source. Actual secrets NEVER live in hub.db, logs, or hub-messages
// (per R52 secrets-handling NEVER-rules).
package hub

import (
	"database/sql"
	"errors"
	"fmt"
	"time"
)

// AgentModelConfig is one row in the agent_model_configs table.
type AgentModelConfig struct {
	AgentID       string    // PK; e.g. "brian" / "rain" / "emma" / "clive" / "coder-template" / "project:<proj>:<agent>"
	Provider      string    // "anthropic" / "deepseek" / "openai" / etc.
	ModelName     string    // "claude-default" / "deepseek-v4-pro" / etc.
	BaseURL       string    // optional; empty for provider-default; e.g. "https://api.deepseek.com/anthropic"
	AuthSecretRef string    // reference-pointer; NEVER actual secret
	Enabled       bool
	CreatedAt     time.Time
	UpdatedAt     time.Time
	Notes         string
}

// ErrAgentModelConfigNotFound is returned when a requested agent_id has no row.
var ErrAgentModelConfigNotFound = errors.New("agent_model_config not found")

// migrateAgentModelConfigs creates the agent_model_configs table + index, then
// seeds default-rows for the canonical bot-hq agents. Idempotent — safe to
// re-run on existing DBs.
func (db *DB) migrateAgentModelConfigs() error {
	schema := `
	CREATE TABLE IF NOT EXISTS agent_model_configs (
		agent_id         TEXT PRIMARY KEY,
		provider         TEXT NOT NULL,
		model_name       TEXT NOT NULL,
		base_url         TEXT NOT NULL DEFAULT '',
		auth_secret_ref  TEXT NOT NULL,
		enabled          INTEGER NOT NULL DEFAULT 1,
		created_at       INTEGER NOT NULL,
		updated_at       INTEGER NOT NULL,
		notes            TEXT NOT NULL DEFAULT ''
	);
	CREATE INDEX IF NOT EXISTS idx_agent_model_configs_enabled ON agent_model_configs(enabled);
	`
	if _, err := db.conn.Exec(schema); err != nil {
		return fmt.Errorf("agent_model_configs schema: %w", err)
	}

	// Seed defaults if table is empty
	var count int
	if err := db.conn.QueryRow("SELECT COUNT(*) FROM agent_model_configs").Scan(&count); err != nil {
		return fmt.Errorf("count agent_model_configs: %w", err)
	}
	if count == 0 {
		for _, cfg := range DefaultAgentModelConfigs() {
			if err := db.SetAgentModelConfig(cfg); err != nil {
				return fmt.Errorf("seed default %s: %w", cfg.AgentID, err)
			}
		}
	}
	return nil
}

// DefaultAgentModelConfigs returns the seed default-rows per phase-t.md v5 R51.
// brian + emma + clive + coder-template default to Claude OAuth (MAX-subscription
// preserved per R43 narrowed). rain defaults to DeepSeek-V4-Pro via Anthropic-
// compatible endpoint (env-var swap pattern; API-key narrow-scoped per
// user msg 17106).
func DefaultAgentModelConfigs() []*AgentModelConfig {
	return []*AgentModelConfig{
		{
			AgentID:       "brian",
			Provider:      "anthropic",
			ModelName:     "claude-default",
			BaseURL:       "",
			AuthSecretRef: "oauth:CLAUDE_CODE_OAUTH_TOKEN",
			Enabled:       true,
			Notes:         "HANDS role; Claude OAuth-MAX-subscription per R43 narrowed",
		},
		{
			AgentID:       "rain",
			Provider:      "deepseek",
			ModelName:     "deepseek-v4-pro",
			BaseURL:       "https://api.deepseek.com/anthropic",
			AuthSecretRef: "env:DEEPSEEK_API_KEY",
			Enabled:       true,
			Notes:         "EYES role; DeepSeek-V4-Pro for cross-model genuine-cognitive-diversity per user msg 17094 + R44 expanded",
		},
		{
			AgentID:       "emma",
			Provider:      "anthropic",
			ModelName:     "claude-haiku-default",
			BaseURL:       "",
			AuthSecretRef: "oauth:CLAUDE_CODE_OAUTH_TOKEN",
			Enabled:       true,
			Notes:         "Discipline-machinery agent; Claude-Haiku for cost-efficiency",
		},
		{
			AgentID:       "clive",
			Provider:      "anthropic",
			ModelName:     "claude-default",
			BaseURL:       "",
			AuthSecretRef: "oauth:CLAUDE_CODE_OAUTH_TOKEN",
			Enabled:       true,
			Notes:         "Voice agent; Claude OAuth-MAX-subscription",
		},
		{
			AgentID:       "coder-template",
			Provider:      "anthropic",
			ModelName:     "claude-default",
			BaseURL:       "",
			AuthSecretRef: "oauth:CLAUDE_CODE_OAUTH_TOKEN",
			Enabled:       true,
			Notes:         "Spawned-coder template; per-spawn override permitted",
		},
	}
}

// GetAgentModelConfig fetches a row by agent_id. Returns ErrAgentModelConfigNotFound
// when no row matches.
func (db *DB) GetAgentModelConfig(agentID string) (*AgentModelConfig, error) {
	var cfg AgentModelConfig
	var enabledInt int
	var createdUnix, updatedUnix int64

	err := db.conn.QueryRow(`
		SELECT agent_id, provider, model_name, base_url, auth_secret_ref, enabled, created_at, updated_at, notes
		FROM agent_model_configs
		WHERE agent_id = ?
	`, agentID).Scan(
		&cfg.AgentID, &cfg.Provider, &cfg.ModelName, &cfg.BaseURL,
		&cfg.AuthSecretRef, &enabledInt, &createdUnix, &updatedUnix, &cfg.Notes,
	)
	if err == sql.ErrNoRows {
		return nil, ErrAgentModelConfigNotFound
	}
	if err != nil {
		return nil, fmt.Errorf("query agent_model_config %s: %w", agentID, err)
	}

	cfg.Enabled = enabledInt != 0
	cfg.CreatedAt = time.Unix(createdUnix, 0).UTC()
	cfg.UpdatedAt = time.Unix(updatedUnix, 0).UTC()
	return &cfg, nil
}

// SetAgentModelConfig upserts a row. CreatedAt is preserved on update; UpdatedAt
// is always set to now. Validates required fields before insert.
func (db *DB) SetAgentModelConfig(cfg *AgentModelConfig) error {
	if cfg.AgentID == "" {
		return errors.New("agent_id is required")
	}
	if cfg.Provider == "" {
		return errors.New("provider is required")
	}
	if cfg.ModelName == "" {
		return errors.New("model_name is required")
	}
	if cfg.AuthSecretRef == "" {
		return errors.New("auth_secret_ref is required (reference-pointer per R52)")
	}

	now := time.Now().UTC()
	createdUnix := now.Unix()
	if !cfg.CreatedAt.IsZero() {
		createdUnix = cfg.CreatedAt.Unix()
	}
	enabledInt := 0
	if cfg.Enabled {
		enabledInt = 1
	}

	// Upsert: ON CONFLICT preserves created_at + updates everything else
	_, err := db.conn.Exec(`
		INSERT INTO agent_model_configs (agent_id, provider, model_name, base_url, auth_secret_ref, enabled, created_at, updated_at, notes)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
		ON CONFLICT(agent_id) DO UPDATE SET
			provider = excluded.provider,
			model_name = excluded.model_name,
			base_url = excluded.base_url,
			auth_secret_ref = excluded.auth_secret_ref,
			enabled = excluded.enabled,
			updated_at = excluded.updated_at,
			notes = excluded.notes
	`,
		cfg.AgentID, cfg.Provider, cfg.ModelName, cfg.BaseURL,
		cfg.AuthSecretRef, enabledInt, createdUnix, now.Unix(), cfg.Notes,
	)
	if err != nil {
		return fmt.Errorf("upsert agent_model_config %s: %w", cfg.AgentID, err)
	}
	return nil
}

// ListAgentModelConfigs returns all rows, optionally filtered to enabled-only.
func (db *DB) ListAgentModelConfigs(enabledOnly bool) ([]*AgentModelConfig, error) {
	q := `SELECT agent_id, provider, model_name, base_url, auth_secret_ref, enabled, created_at, updated_at, notes FROM agent_model_configs`
	if enabledOnly {
		q += " WHERE enabled = 1"
	}
	q += " ORDER BY agent_id"

	rows, err := db.conn.Query(q)
	if err != nil {
		return nil, fmt.Errorf("query agent_model_configs: %w", err)
	}
	defer rows.Close()

	var configs []*AgentModelConfig
	for rows.Next() {
		var cfg AgentModelConfig
		var enabledInt int
		var createdUnix, updatedUnix int64
		if err := rows.Scan(
			&cfg.AgentID, &cfg.Provider, &cfg.ModelName, &cfg.BaseURL,
			&cfg.AuthSecretRef, &enabledInt, &createdUnix, &updatedUnix, &cfg.Notes,
		); err != nil {
			return nil, fmt.Errorf("scan: %w", err)
		}
		cfg.Enabled = enabledInt != 0
		cfg.CreatedAt = time.Unix(createdUnix, 0).UTC()
		cfg.UpdatedAt = time.Unix(updatedUnix, 0).UTC()
		configs = append(configs, &cfg)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("rows iteration: %w", err)
	}
	return configs, nil
}

// DeleteAgentModelConfig removes a row by agent_id. Returns nil even if no
// row matched (idempotent delete).
func (db *DB) DeleteAgentModelConfig(agentID string) error {
	_, err := db.conn.Exec("DELETE FROM agent_model_configs WHERE agent_id = ?", agentID)
	if err != nil {
		return fmt.Errorf("delete agent_model_config %s: %w", agentID, err)
	}
	return nil
}

// IsDefaultSeed returns true if agent_id matches a default-seed row. Default-seed
// rows can be reset via bot-hq config reset but should not be deleted.
func IsDefaultSeed(agentID string) bool {
	for _, cfg := range DefaultAgentModelConfigs() {
		if cfg.AgentID == agentID {
			return true
		}
	}
	return false
}
