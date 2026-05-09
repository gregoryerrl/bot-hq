package providers

import (
	"errors"
	"testing"
)

// ====== Lookup + IsKnown ======

func TestLookup_anthropic(t *testing.T) {
	p, err := Lookup("anthropic")
	if err != nil {
		t.Fatalf("Lookup anthropic: %v", err)
	}
	if p.AuthMode != AuthOAuth {
		t.Errorf("anthropic auth-mode = %q, want oauth", p.AuthMode)
	}
	if !p.AnthropicCompat {
		t.Error("anthropic should be Anthropic-compat by definition")
	}
}

func TestLookup_deepseek(t *testing.T) {
	p, err := Lookup("deepseek")
	if err != nil {
		t.Fatalf("Lookup deepseek: %v", err)
	}
	if p.DefaultEndpoint != "https://api.deepseek.com/anthropic" {
		t.Errorf("deepseek endpoint = %q", p.DefaultEndpoint)
	}
	if !p.AnthropicCompat {
		t.Error("deepseek should be Anthropic-compat per TESTS 1-7 capability-parity validation")
	}
}

func TestLookup_unknownReturnsError(t *testing.T) {
	_, err := Lookup("imaginary-provider-xyz")
	if err == nil {
		t.Fatal("expected error for unknown provider")
	}
	if !errors.Is(err, ErrUnknownProvider) {
		t.Errorf("err = %v, want errors.Is(err, ErrUnknownProvider)", err)
	}
}

func TestIsKnown_recognizesRegistered(t *testing.T) {
	cases := []struct {
		name string
		want bool
	}{
		{"anthropic", true},
		{"deepseek", true},
		{"openai", true},
		{"google-vertex", true},
		{"azure", true},
		{"mistral", true},
		{"cohere", true},
		{"together-ai", true},
		{"imaginary-xyz", false},
		{"", false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if got := IsKnown(c.name); got != c.want {
				t.Errorf("IsKnown(%q) = %v, want %v", c.name, got, c.want)
			}
		})
	}
}

// ====== All + AnthropicCompatProviders ======

func TestAll_returnsSortedByName(t *testing.T) {
	all := All()
	if len(all) != 8 {
		t.Errorf("len(All()) = %d, want 8 (registry size)", len(all))
	}
	for i := 1; i < len(all); i++ {
		if all[i-1].Name >= all[i].Name {
			t.Errorf("All() not sorted: %q before %q", all[i-1].Name, all[i].Name)
		}
	}
}

func TestAnthropicCompatProviders_filtersToCompat(t *testing.T) {
	compat := AnthropicCompatProviders()
	if len(compat) < 2 {
		t.Errorf("expected ≥2 Anthropic-compat providers (anthropic + deepseek), got %d", len(compat))
	}
	for _, p := range compat {
		if !p.AnthropicCompat {
			t.Errorf("AnthropicCompatProviders contained non-compat: %+v", p)
		}
	}
}

// ====== Validate ======

func TestValidate_anthropicOAuth(t *testing.T) {
	if err := Validate("anthropic", "claude-default", "", "oauth:CLAUDE_CODE_OAUTH_TOKEN"); err != nil {
		t.Errorf("Validate anthropic+oauth should pass: %v", err)
	}
}

func TestValidate_deepseekEnvKey(t *testing.T) {
	if err := Validate("deepseek", "deepseek-v4-pro", "https://api.deepseek.com/anthropic", "env:DEEPSEEK_API_KEY"); err != nil {
		t.Errorf("Validate deepseek+env should pass: %v", err)
	}
}

func TestValidate_unknownProviderRejected(t *testing.T) {
	err := Validate("imaginary-zzz", "x", "", "env:X")
	if err == nil {
		t.Fatal("expected error for unknown provider")
	}
	if !errors.Is(err, ErrUnknownProvider) {
		t.Errorf("err = %v, want errors.Is ErrUnknownProvider", err)
	}
}

func TestValidate_oauthSchemeOnNonOAuthProviderRejected(t *testing.T) {
	// deepseek is api-key auth-mode; oauth: scheme is mismatch
	if err := Validate("deepseek", "deepseek-v4-pro", "", "oauth:WRONG"); err == nil {
		t.Error("expected error: oauth scheme on api-key provider")
	}
}

func TestValidate_emptyAuthSecretRejected(t *testing.T) {
	if err := Validate("anthropic", "claude-default", "", ""); err == nil {
		t.Error("expected error: empty auth_secret_ref")
	}
}

func TestValidate_serviceAccountSchemeOnGoogleVertex(t *testing.T) {
	if err := Validate("google-vertex", "gemini-1.5-pro", "https://us-central1-aiplatform.googleapis.com/v1", "sa:/path/to/service-account.json"); err != nil {
		t.Errorf("Validate google-vertex+sa should pass: %v", err)
	}
}

func TestValidate_emptyProviderRejected(t *testing.T) {
	if err := Validate("", "x", "", "env:X"); err == nil {
		t.Error("expected error for empty provider")
	}
}

func TestValidate_envSchemeWorksForAllAuthModes(t *testing.T) {
	cases := []struct{ provider, model, baseURL, authRef string }{
		{"anthropic", "claude-default", "", "env:X"},
		{"deepseek", "deepseek-v4-pro", "https://api.deepseek.com/anthropic", "env:X"},
		{"google-vertex", "gemini-1.5-pro", "https://us-central1-aiplatform.googleapis.com/v1", "env:X"},
	}
	for _, c := range cases {
		if err := Validate(c.provider, c.model, c.baseURL, c.authRef); err != nil {
			t.Errorf("env-scheme should work on %s, got: %v", c.provider, err)
		}
	}
}

// ====== validAuthSchemeForMode (internal helper smoke) ======

func TestValidAuthSchemeForMode_acceptsKnownSchemes(t *testing.T) {
	cases := []struct {
		ref  string
		mode AuthMode
		want bool
	}{
		{"oauth:X", AuthOAuth, true},
		{"oauth:X", AuthAPIKey, false}, // mismatch
		{"env:X", AuthOAuth, true},     // env always works
		{"env:X", AuthAPIKey, true},
		{"env:X", AuthServiceAccount, true},
		{"file:X", AuthAPIKey, true},
		{"sa:X", AuthServiceAccount, true},
		{"sa:X", AuthAPIKey, false}, // mismatch
		{"unknown:X", AuthAPIKey, false},
		{"", AuthAPIKey, false}, // empty rejected
	}
	for _, c := range cases {
		t.Run(c.ref+"-"+string(c.mode), func(t *testing.T) {
			if got := validAuthSchemeForMode(c.ref, c.mode); got != c.want {
				t.Errorf("validAuthSchemeForMode(%q, %q) = %v, want %v", c.ref, c.mode, got, c.want)
			}
		})
	}
}
