// Package agentconfig resolves per-agent model-config from hub.db
// agent_model_configs (R51 + R52 per phase-t.md v5 T-1.4) and produces the
// env-var list to inject at agent-spawn-time. Implements the reference-pointer
// secret-storage strategy: actual secrets resolve from env-var/keychain/.env
// at spawn-time; hub.db only stores the reference-pointer.
//
// Reference-pointer schemes:
//
//	oauth:<env-var-name>       - subprocess inherits env-var directly (Claude path)
//	env:<env-var-name>         - resolve via os.Getenv() at spawn-time
//	keychain:<id>              - resolve via macOS Keychain (security CLI)
//	file:<path>#<key>          - resolve from gitignored .env-style file
//
// R43 narrowed: env-var-list is empty for Claude path (subprocess inherits
// CLAUDE_CODE_OAUTH_TOKEN from env). For non-Claude providers, returns
// ANTHROPIC_BASE_URL + ANTHROPIC_AUTH_TOKEN + ANTHROPIC_MODEL env-var-swap.
package agentconfig

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/gregoryerrl/bot-hq/internal/hub"
)

// ErrSecretUnresolvable is returned when an auth_secret_ref cannot be resolved
// (env-var unset / keychain-id missing / file-not-found / unknown scheme).
// Callers MUST surface this to the user via emma alert + structured-log; the
// agent MUST NOT spawn with degraded-config silently.
var ErrSecretUnresolvable = errors.New("auth_secret_ref unresolvable")

// EnvVar is one entry in the env-var-list returned by BuildSpawnEnv.
type EnvVar struct {
	Name  string
	Value string // actual secret value for non-oauth schemes; secret REDACTED in String()
}

// String returns "NAME=<REDACTED>" for any env-var carrying a secret value.
// Use Format() to obtain the unredacted "NAME=VALUE" form for subprocess
// invocation. This split prevents accidental log-leakage per R52.
func (e EnvVar) String() string {
	return fmt.Sprintf("%s=<REDACTED>", e.Name)
}

// Format returns "NAME=VALUE" for direct subprocess env-var injection. NEVER
// log this output (carries actual secret) per R52 secrets-handling NEVER-rules.
func (e EnvVar) Format() string {
	return fmt.Sprintf("%s=%s", e.Name, e.Value)
}

// BuildSpawnEnv resolves the agent's model-config + secret-ref and returns the
// env-var-list to inject at subprocess-spawn-time. Returns empty slice for
// Claude OAuth path (subprocess inherits CLAUDE_CODE_OAUTH_TOKEN from env, no
// explicit injection needed).
//
// For non-Claude providers (e.g. DeepSeek), returns:
//   - ANTHROPIC_BASE_URL=<base_url>
//   - ANTHROPIC_AUTH_TOKEN=<resolved_secret>
//   - ANTHROPIC_MODEL=<model_name>
//
// This env-var-swap pattern preserves R43 AROUND-CC subprocess discipline
// (claude CLI honors these env-vars to redirect backend; subprocess pattern
// unchanged).
func BuildSpawnEnv(cfg *hub.AgentModelConfig) ([]EnvVar, error) {
	if cfg == nil {
		return nil, errors.New("nil agent config")
	}
	if !cfg.Enabled {
		return nil, fmt.Errorf("agent config %s is disabled", cfg.AgentID)
	}

	// Claude OAuth path: subprocess inherits env-var directly; no explicit injection.
	if isClaudeOAuthPath(cfg) {
		return nil, nil
	}

	// Non-Claude path: resolve secret + emit env-var swap
	secret, err := ResolveSecret(cfg.AuthSecretRef)
	if err != nil {
		return nil, fmt.Errorf("%w: %s for agent %s: %v", ErrSecretUnresolvable, cfg.AuthSecretRef, cfg.AgentID, err)
	}

	envs := []EnvVar{
		{Name: "ANTHROPIC_AUTH_TOKEN", Value: secret},
		{Name: "ANTHROPIC_MODEL", Value: cfg.ModelName},
	}
	if cfg.BaseURL != "" {
		envs = append(envs, EnvVar{Name: "ANTHROPIC_BASE_URL", Value: cfg.BaseURL})
	}
	return envs, nil
}

// isClaudeOAuthPath returns true when the config matches the Claude OAuth
// subscription pattern (provider=anthropic + oauth: secret-ref + empty base_url).
// In this case the subprocess inherits CLAUDE_CODE_OAUTH_TOKEN from env without
// explicit injection.
func isClaudeOAuthPath(cfg *hub.AgentModelConfig) bool {
	return cfg.Provider == "anthropic" &&
		strings.HasPrefix(cfg.AuthSecretRef, "oauth:") &&
		cfg.BaseURL == ""
}

// ResolveSecret resolves a reference-pointer to its actual secret value by
// scheme. Returns error if the reference cannot be resolved.
//
// Schemes:
//   - oauth:<VAR>       — read os.Getenv(VAR)
//   - env:<VAR>         — read os.Getenv(VAR)
//   - keychain:<ID>     — invoke `security find-generic-password -s ID -w`
//   - file:<PATH>#<KEY> — read PATH (.env-style) + extract KEY=VALUE line
//
// All schemes return the actual secret string. Caller MUST NOT log the result
// per R52 secrets-handling NEVER-rules.
func ResolveSecret(secretRef string) (string, error) {
	if secretRef == "" {
		return "", errors.New("empty secret-ref")
	}

	scheme, value, found := strings.Cut(secretRef, ":")
	if !found {
		return "", fmt.Errorf("invalid secret-ref format (expected scheme:value): %s", secretRef)
	}

	switch scheme {
	case "oauth", "env":
		val := os.Getenv(value)
		if val == "" {
			return "", fmt.Errorf("env-var %s is unset", value)
		}
		return val, nil

	case "keychain":
		return resolveKeychainSecret(value)

	case "file":
		return resolveFileSecret(value)

	default:
		return "", fmt.Errorf("unknown secret-ref scheme: %s", scheme)
	}
}

// resolveKeychainSecret invokes the macOS Keychain CLI to retrieve a generic
// password by service-id. Returns the password value on success.
//
// Implementation detail: uses `security find-generic-password -s <id> -w`
// which is a one-shot lookup. CLI is macOS-only; on other platforms this
// function returns ErrSecretUnresolvable.
func resolveKeychainSecret(serviceID string) (string, error) {
	if serviceID == "" {
		return "", errors.New("empty keychain service-id")
	}
	cmd := exec.Command("security", "find-generic-password", "-s", serviceID, "-w")
	out, err := cmd.Output()
	if err != nil {
		return "", fmt.Errorf("keychain lookup for %s: %w", serviceID, err)
	}
	return strings.TrimSpace(string(out)), nil
}

// resolveFileSecret reads a .env-style file at PATH and extracts the value
// for KEY. Format: PATH#KEY where PATH may include ~ for home expansion.
//
// File format: lines of "KEY=VALUE"; comments (#) and blank lines ignored.
// Per R52 secrets-handling NEVER-rules: file MUST be gitignored + mode 0600.
func resolveFileSecret(spec string) (string, error) {
	path, key, found := strings.Cut(spec, "#")
	if !found {
		return "", fmt.Errorf("file secret-ref missing #KEY suffix: %s", spec)
	}
	if path == "" {
		return "", errors.New("empty file path")
	}
	if key == "" {
		return "", errors.New("empty file key")
	}

	// Expand ~ to home directory
	if strings.HasPrefix(path, "~/") {
		home, err := os.UserHomeDir()
		if err != nil {
			return "", fmt.Errorf("home dir: %w", err)
		}
		path = filepath.Join(home, path[2:])
	}

	data, err := os.ReadFile(path)
	if err != nil {
		return "", fmt.Errorf("read %s: %w", path, err)
	}

	for _, line := range strings.Split(string(data), "\n") {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" || strings.HasPrefix(trimmed, "#") {
			continue
		}
		k, v, ok := strings.Cut(trimmed, "=")
		if !ok {
			continue
		}
		if strings.TrimSpace(k) == key {
			// Strip surrounding quotes if present
			val := strings.TrimSpace(v)
			val = strings.TrimPrefix(val, `"`)
			val = strings.TrimSuffix(val, `"`)
			val = strings.TrimPrefix(val, `'`)
			val = strings.TrimSuffix(val, `'`)
			if val == "" {
				return "", fmt.Errorf("key %s found in %s but value is empty", key, path)
			}
			return val, nil
		}
	}
	return "", fmt.Errorf("key %s not found in %s", key, path)
}

// FormatTmuxEnvArgs formats env-var-list as `-e KEY=VALUE` arg pairs for
// `tmux new-session`. Returns empty slice for empty input. Caller appends
// these args to the tmux invocation.
func FormatTmuxEnvArgs(envs []EnvVar) []string {
	args := make([]string, 0, len(envs)*2)
	for _, e := range envs {
		args = append(args, "-e", e.Format())
	}
	return args
}
