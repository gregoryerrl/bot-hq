package agentconfig

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/hub"
)

func TestBuildSpawnEnv_claudeOAuthPath_returnsEmpty(t *testing.T) {
	cfg := &hub.AgentModelConfig{
		AgentID:       "brian",
		Provider:      "anthropic",
		ModelName:     "claude-default",
		BaseURL:       "",
		AuthSecretRef: "oauth:CLAUDE_CODE_OAUTH_TOKEN",
		Enabled:       true,
	}
	envs, err := BuildSpawnEnv(cfg)
	if err != nil {
		t.Fatalf("BuildSpawnEnv: %v", err)
	}
	if len(envs) != 0 {
		t.Errorf("Claude OAuth path should produce empty env-list, got %v", envs)
	}
}

func TestBuildSpawnEnv_deepseekPath_envVarSwap(t *testing.T) {
	t.Setenv("DEEPSEEK_API_KEY", "sk-test-deepseek-key-do-not-leak")

	cfg := &hub.AgentModelConfig{
		AgentID:       "rain",
		Provider:      "deepseek",
		ModelName:     "deepseek-v4-pro",
		BaseURL:       "https://api.deepseek.com/anthropic",
		AuthSecretRef: "env:DEEPSEEK_API_KEY",
		Enabled:       true,
	}
	envs, err := BuildSpawnEnv(cfg)
	if err != nil {
		t.Fatalf("BuildSpawnEnv: %v", err)
	}
	if len(envs) != 3 {
		t.Fatalf("DeepSeek path should produce 3 env-vars (BASE_URL + AUTH_TOKEN + MODEL), got %d: %v", len(envs), envs)
	}

	// Verify env-var names + values present (order-independent)
	got := map[string]string{}
	for _, e := range envs {
		got[e.Name] = e.Value
	}
	if got["ANTHROPIC_BASE_URL"] != "https://api.deepseek.com/anthropic" {
		t.Errorf("ANTHROPIC_BASE_URL = %q, want https://api.deepseek.com/anthropic", got["ANTHROPIC_BASE_URL"])
	}
	if got["ANTHROPIC_AUTH_TOKEN"] != "sk-test-deepseek-key-do-not-leak" {
		t.Errorf("ANTHROPIC_AUTH_TOKEN value mismatch (REDACTED here for security)")
	}
	if got["ANTHROPIC_MODEL"] != "deepseek-v4-pro" {
		t.Errorf("ANTHROPIC_MODEL = %q, want deepseek-v4-pro", got["ANTHROPIC_MODEL"])
	}
}

func TestBuildSpawnEnv_disabledConfig_errors(t *testing.T) {
	cfg := &hub.AgentModelConfig{
		AgentID:       "rain",
		Provider:      "deepseek",
		ModelName:     "deepseek-v4-pro",
		BaseURL:       "https://api.deepseek.com/anthropic",
		AuthSecretRef: "env:DEEPSEEK_API_KEY",
		Enabled:       false,
	}
	_, err := BuildSpawnEnv(cfg)
	if err == nil {
		t.Error("disabled config should error")
	}
}

func TestBuildSpawnEnv_unresolvableSecret_returnsErrSentinel(t *testing.T) {
	t.Setenv("DEEPSEEK_API_KEY_NONEXISTENT", "")
	os.Unsetenv("DEEPSEEK_API_KEY_NONEXISTENT") // force unset

	cfg := &hub.AgentModelConfig{
		AgentID:       "test",
		Provider:      "deepseek",
		ModelName:     "deepseek-v4-pro",
		AuthSecretRef: "env:DEEPSEEK_API_KEY_NONEXISTENT",
		Enabled:       true,
	}
	_, err := BuildSpawnEnv(cfg)
	if err == nil {
		t.Fatal("expected error for unresolvable secret")
	}
	if !errors.Is(err, ErrSecretUnresolvable) {
		t.Errorf("expected ErrSecretUnresolvable wrapping, got: %v", err)
	}
}

func TestResolveSecret_envScheme(t *testing.T) {
	t.Setenv("MY_TEST_SECRET", "the-actual-secret")
	got, err := ResolveSecret("env:MY_TEST_SECRET")
	if err != nil {
		t.Fatalf("ResolveSecret env: %v", err)
	}
	if got != "the-actual-secret" {
		t.Errorf("got %q, want the-actual-secret", got)
	}
}

func TestResolveSecret_oauthScheme(t *testing.T) {
	t.Setenv("CLAUDE_CODE_OAUTH_TOKEN", "oauth-token-value")
	got, err := ResolveSecret("oauth:CLAUDE_CODE_OAUTH_TOKEN")
	if err != nil {
		t.Fatalf("ResolveSecret oauth: %v", err)
	}
	if got != "oauth-token-value" {
		t.Errorf("got %q, want oauth-token-value", got)
	}
}

func TestResolveSecret_envUnset_errors(t *testing.T) {
	os.Unsetenv("DEFINITELY_UNSET_VAR_XYZ")
	_, err := ResolveSecret("env:DEFINITELY_UNSET_VAR_XYZ")
	if err == nil {
		t.Error("expected error for unset env-var")
	}
}

func TestResolveSecret_unknownScheme_errors(t *testing.T) {
	_, err := ResolveSecret("unknown-scheme:some-value")
	if err == nil {
		t.Error("expected error for unknown scheme")
	}
}

func TestResolveSecret_invalidFormat_errors(t *testing.T) {
	_, err := ResolveSecret("no-colon-here")
	if err == nil {
		t.Error("expected error for missing colon")
	}
}

func TestResolveSecret_emptyRef_errors(t *testing.T) {
	_, err := ResolveSecret("")
	if err == nil {
		t.Error("expected error for empty ref")
	}
}

func TestResolveFileSecret_basicLookup(t *testing.T) {
	dir := t.TempDir()
	envPath := filepath.Join(dir, ".env")
	envContent := `# comment
DEEPSEEK_API_KEY=sk-test-from-file
OTHER_VAR=ignore-me
`
	if err := os.WriteFile(envPath, []byte(envContent), 0o600); err != nil {
		t.Fatalf("write test .env: %v", err)
	}

	got, err := ResolveSecret("file:" + envPath + "#DEEPSEEK_API_KEY")
	if err != nil {
		t.Fatalf("ResolveSecret file: %v", err)
	}
	if got != "sk-test-from-file" {
		t.Errorf("got %q, want sk-test-from-file", got)
	}
}

func TestResolveFileSecret_quotedValue(t *testing.T) {
	dir := t.TempDir()
	envPath := filepath.Join(dir, ".env")
	envContent := `QUOTED_DBL="value with spaces"
QUOTED_SGL='single-quoted'
`
	if err := os.WriteFile(envPath, []byte(envContent), 0o600); err != nil {
		t.Fatalf("write test .env: %v", err)
	}

	got, err := ResolveSecret("file:" + envPath + "#QUOTED_DBL")
	if err != nil {
		t.Fatalf("ResolveSecret file dbl: %v", err)
	}
	if got != "value with spaces" {
		t.Errorf("dbl got %q, want value with spaces", got)
	}

	got, err = ResolveSecret("file:" + envPath + "#QUOTED_SGL")
	if err != nil {
		t.Fatalf("ResolveSecret file sgl: %v", err)
	}
	if got != "single-quoted" {
		t.Errorf("sgl got %q, want single-quoted", got)
	}
}

func TestResolveFileSecret_keyNotFound_errors(t *testing.T) {
	dir := t.TempDir()
	envPath := filepath.Join(dir, ".env")
	if err := os.WriteFile(envPath, []byte("FOO=bar\n"), 0o600); err != nil {
		t.Fatalf("write: %v", err)
	}
	_, err := ResolveSecret("file:" + envPath + "#MISSING_KEY")
	if err == nil {
		t.Error("expected error for missing key")
	}
}

func TestResolveFileSecret_fileMissing_errors(t *testing.T) {
	_, err := ResolveSecret("file:/nonexistent/path/.env#KEY")
	if err == nil {
		t.Error("expected error for missing file")
	}
}

func TestResolveFileSecret_missingHashKey_errors(t *testing.T) {
	dir := t.TempDir()
	envPath := filepath.Join(dir, ".env")
	if err := os.WriteFile(envPath, []byte("FOO=bar\n"), 0o600); err != nil {
		t.Fatalf("write: %v", err)
	}
	_, err := ResolveSecret("file:" + envPath)
	if err == nil {
		t.Error("expected error for missing #KEY suffix")
	}
}

func TestEnvVar_StringRedactsSecret(t *testing.T) {
	e := EnvVar{Name: "ANTHROPIC_AUTH_TOKEN", Value: "sk-secret-do-not-leak"}
	str := e.String()
	if strings.Contains(str, "sk-secret-do-not-leak") {
		t.Errorf("String() leaked secret: %s", str)
	}
	if !strings.Contains(str, "REDACTED") {
		t.Errorf("String() should contain REDACTED, got: %s", str)
	}
}

func TestEnvVar_FormatExposesSecret(t *testing.T) {
	e := EnvVar{Name: "ANTHROPIC_AUTH_TOKEN", Value: "sk-secret-for-subprocess"}
	formatted := e.Format()
	want := "ANTHROPIC_AUTH_TOKEN=sk-secret-for-subprocess"
	if formatted != want {
		t.Errorf("Format() = %q, want %q", formatted, want)
	}
}

func TestFormatTmuxEnvArgs(t *testing.T) {
	envs := []EnvVar{
		{Name: "ANTHROPIC_BASE_URL", Value: "https://api.deepseek.com/anthropic"},
		{Name: "ANTHROPIC_MODEL", Value: "deepseek-v4-pro"},
	}
	args := FormatTmuxEnvArgs(envs)
	want := []string{
		"-e", "ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic",
		"-e", "ANTHROPIC_MODEL=deepseek-v4-pro",
	}
	if len(args) != len(want) {
		t.Fatalf("len(args) = %d, want %d", len(args), len(want))
	}
	for i, a := range args {
		if a != want[i] {
			t.Errorf("args[%d] = %q, want %q", i, a, want[i])
		}
	}
}

func TestFormatTmuxEnvArgs_emptyInput(t *testing.T) {
	args := FormatTmuxEnvArgs(nil)
	if len(args) != 0 {
		t.Errorf("empty input should produce empty args, got: %v", args)
	}
}
