// bot-hq config CLI command (Phase T T-1.2 per phase-t.md v5).
//
// Surfaces hub.db agent_model_configs CRUD via CLI per R52 HUB-DB-CONFIG-DISCIPLINE.
// Web UI settings tab (T-1.3) shares the same hub.db backend; CLI-vs-UI parity.
//
// Subcommands:
//
//	bot-hq config show <agent-id>
//	bot-hq config list [--enabled-only]
//	bot-hq config set <agent-id> <field> <value>
//	bot-hq config validate [<agent-id>]
//	bot-hq config reset <agent-id>
//
// Per R52 secrets-handling NEVER-rules: actual-secret NEVER displayed (auth_secret_ref
// shown as reference-pointer-name only). Validate exercises secret-resolution but
// only reports success/failure (NEVER prints actual secret).
package main

import (
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"strings"

	"github.com/gregoryerrl/bot-hq/internal/agentconfig"
	"github.com/gregoryerrl/bot-hq/internal/hub"
)

func runConfig(args []string) {
	if len(args) == 0 {
		printConfigUsage()
		os.Exit(2)
	}

	db, err := openHubDBForCLI()
	if err != nil {
		fmt.Fprintf(os.Stderr, "open hub.db: %v\n", err)
		os.Exit(1)
	}
	defer db.Close()

	cmd := args[0]
	rest := args[1:]

	switch cmd {
	case "show":
		runConfigShow(db, rest)
	case "list":
		runConfigList(db, rest)
	case "set":
		runConfigSet(db, rest)
	case "validate":
		runConfigValidate(db, rest)
	case "reset":
		runConfigReset(db, rest)
	case "help", "-h", "--help":
		printConfigUsage()
	default:
		fmt.Fprintf(os.Stderr, "unknown config subcommand: %s\n\n", cmd)
		printConfigUsage()
		os.Exit(2)
	}
}

func openHubDBForCLI() (*hub.DB, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return nil, fmt.Errorf("home dir: %w", err)
	}
	return hub.OpenDB(filepath.Join(home, ".bot-hq", "hub.db"))
}

func printConfigUsage() {
	fmt.Println(`bot-hq config — per-agent model-config CRUD (R51 + R52)

Usage:
  bot-hq config show <agent-id>
  bot-hq config list [--enabled-only]
  bot-hq config set <agent-id> <field> <value>
  bot-hq config validate [<agent-id>]
  bot-hq config reset <agent-id>

Fields (for 'set'):
  provider          anthropic | deepseek | openai | ...
  model             model name (e.g. claude-default, deepseek-v4-pro)
  base_url          provider endpoint (empty for default)
  auth_secret_ref   reference-pointer (oauth:VAR | env:VAR | keychain:ID | file:PATH#KEY)
  enabled           true | false
  notes             free-form description

Per R52 secrets-handling NEVER-rules: actual secrets NEVER stored in hub.db,
NEVER printed by show/list, NEVER logged. auth_secret_ref is a reference-
pointer; the actual secret resolves at agent-spawn-time from env-var,
keychain, or .env file.

Examples:
  bot-hq config show rain
  bot-hq config list --enabled-only
  bot-hq config set rain auth_secret_ref env:DEEPSEEK_API_KEY
  bot-hq config set rain model deepseek-v4-pro
  bot-hq config validate rain
  bot-hq config reset rain`)
}

func runConfigShow(db *hub.DB, args []string) {
	if len(args) != 1 {
		fmt.Fprintln(os.Stderr, "usage: bot-hq config show <agent-id>")
		os.Exit(2)
	}
	cfg, err := db.GetAgentModelConfig(args[0])
	if err != nil {
		fmt.Fprintf(os.Stderr, "%v\n", err)
		os.Exit(1)
	}
	printConfigRow(cfg)
}

func runConfigList(db *hub.DB, args []string) {
	enabledOnly := false
	for _, a := range args {
		if a == "--enabled-only" {
			enabledOnly = true
		}
	}
	configs, err := db.ListAgentModelConfigs(enabledOnly)
	if err != nil {
		fmt.Fprintf(os.Stderr, "list: %v\n", err)
		os.Exit(1)
	}
	if len(configs) == 0 {
		fmt.Println("(no agent_model_configs rows)")
		return
	}
	fmt.Printf("%-32s %-12s %-24s %-8s %s\n", "AGENT-ID", "PROVIDER", "MODEL", "ENABLED", "AUTH-REF (reference-pointer; secret REDACTED)")
	for _, c := range configs {
		enabled := "yes"
		if !c.Enabled {
			enabled = "no"
		}
		fmt.Printf("%-32s %-12s %-24s %-8s %s\n", c.AgentID, c.Provider, c.ModelName, enabled, c.AuthSecretRef)
	}
}

func runConfigSet(db *hub.DB, args []string) {
	if len(args) != 3 {
		fmt.Fprintln(os.Stderr, "usage: bot-hq config set <agent-id> <field> <value>")
		os.Exit(2)
	}
	agentID, field, value := args[0], args[1], args[2]

	// Get existing or create new
	cfg, err := db.GetAgentModelConfig(agentID)
	if err != nil {
		// New row: require all required fields supplied via subsequent set commands
		// Minimal-default for new row
		cfg = &hub.AgentModelConfig{
			AgentID:       agentID,
			Provider:      "anthropic",
			ModelName:     "claude-default",
			AuthSecretRef: "oauth:CLAUDE_CODE_OAUTH_TOKEN",
			Enabled:       true,
		}
	}

	switch field {
	case "provider":
		cfg.Provider = value
	case "model":
		cfg.ModelName = value
	case "base_url":
		// Optional URL validation — allow empty + accept reasonable HTTPS URL
		if value != "" {
			if _, urlErr := url.Parse(value); urlErr != nil {
				fmt.Fprintf(os.Stderr, "invalid base_url: %v\n", urlErr)
				os.Exit(2)
			}
		}
		cfg.BaseURL = value
	case "auth_secret_ref":
		cfg.AuthSecretRef = value
	case "enabled":
		switch strings.ToLower(value) {
		case "true", "yes", "1":
			cfg.Enabled = true
		case "false", "no", "0":
			cfg.Enabled = false
		default:
			fmt.Fprintf(os.Stderr, "enabled must be true|false, got: %s\n", value)
			os.Exit(2)
		}
	case "notes":
		cfg.Notes = value
	default:
		fmt.Fprintf(os.Stderr, "unknown field: %s (try: provider, model, base_url, auth_secret_ref, enabled, notes)\n", field)
		os.Exit(2)
	}

	if err := db.SetAgentModelConfig(cfg); err != nil {
		fmt.Fprintf(os.Stderr, "set: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("OK: agent_model_config %s.%s updated\n", agentID, field)
}

func runConfigValidate(db *hub.DB, args []string) {
	var configs []*hub.AgentModelConfig
	if len(args) == 1 {
		cfg, err := db.GetAgentModelConfig(args[0])
		if err != nil {
			fmt.Fprintf(os.Stderr, "%v\n", err)
			os.Exit(1)
		}
		configs = []*hub.AgentModelConfig{cfg}
	} else {
		var err error
		configs, err = db.ListAgentModelConfigs(true)
		if err != nil {
			fmt.Fprintf(os.Stderr, "list: %v\n", err)
			os.Exit(1)
		}
	}

	allValid := true
	for _, cfg := range configs {
		valid, msg := validateOneConfig(cfg)
		status := "VALID"
		if !valid {
			status = "INVALID"
			allValid = false
		}
		fmt.Printf("%-32s [%s] %s\n", cfg.AgentID, status, msg)
	}
	if !allValid {
		os.Exit(1)
	}
}

// validateOneConfig exercises secret-resolution + reports valid/invalid.
// Returns (valid, human-readable message). NEVER returns actual secret.
func validateOneConfig(cfg *hub.AgentModelConfig) (bool, string) {
	if !cfg.Enabled {
		return true, "disabled (skipped)"
	}

	// Validate secret-ref resolves
	_, err := agentconfig.ResolveSecret(cfg.AuthSecretRef)
	if err != nil {
		return false, fmt.Sprintf("secret-ref %s unresolvable: %v", cfg.AuthSecretRef, err)
	}

	// Note: endpoint reachability check (HTTPS handshake) is deferred to T-0.5
	// capability-parity test scripts at scripts/capability-parity/run-all.sh
	// (requires DEEPSEEK_API_KEY env-var; live-fire validation post-key-rotation).
	// Validate-CLI does the static-resolution check only.

	return true, fmt.Sprintf("provider=%s model=%s base_url=%s secret-ref-resolves=ok", cfg.Provider, cfg.ModelName, displayBaseURL(cfg.BaseURL))
}

func displayBaseURL(url string) string {
	if url == "" {
		return "(provider-default)"
	}
	return url
}

func runConfigReset(db *hub.DB, args []string) {
	if len(args) != 1 {
		fmt.Fprintln(os.Stderr, "usage: bot-hq config reset <agent-id>")
		os.Exit(2)
	}
	agentID := args[0]

	if !hub.IsDefaultSeed(agentID) {
		fmt.Fprintf(os.Stderr, "%s is not a default-seed agent (use 'bot-hq config set' to modify, or '... config delete-class' for non-default rows)\n", agentID)
		os.Exit(2)
	}

	// Find the default row in DefaultAgentModelConfigs
	for _, def := range hub.DefaultAgentModelConfigs() {
		if def.AgentID == agentID {
			if err := db.SetAgentModelConfig(def); err != nil {
				fmt.Fprintf(os.Stderr, "reset: %v\n", err)
				os.Exit(1)
			}
			fmt.Printf("OK: %s reset to default-seed values\n", agentID)
			return
		}
	}
}

func printConfigRow(cfg *hub.AgentModelConfig) {
	enabled := "yes"
	if !cfg.Enabled {
		enabled = "no"
	}
	fmt.Printf("agent_id:        %s\n", cfg.AgentID)
	fmt.Printf("provider:        %s\n", cfg.Provider)
	fmt.Printf("model_name:      %s\n", cfg.ModelName)
	fmt.Printf("base_url:        %s\n", displayBaseURL(cfg.BaseURL))
	fmt.Printf("auth_secret_ref: %s   (reference-pointer; actual secret REDACTED per R52)\n", cfg.AuthSecretRef)
	fmt.Printf("enabled:         %s\n", enabled)
	fmt.Printf("notes:           %s\n", cfg.Notes)
	fmt.Printf("created_at:      %s\n", cfg.CreatedAt.Format("2006-01-02T15:04:05Z"))
	fmt.Printf("updated_at:      %s\n", cfg.UpdatedAt.Format("2006-01-02T15:04:05Z"))
}
