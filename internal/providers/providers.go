// Package providers enumerates known LLM providers + their endpoint
// conventions for bot-hq's per-agent-model-config infrastructure
// (phase-t.md v5 R51 + R52 + T-8.6).
//
// Multi-provider routing builds on the env-var-swap pattern in
// internal/agentconfig: ANTHROPIC_BASE_URL + ANTHROPIC_AUTH_TOKEN +
// ANTHROPIC_MODEL injected at agent-spawn time. Providers with
// AnthropicCompat=true accept this swap directly. Providers without
// AnthropicCompat require additional adaptation (deferred — see
// T-8.6-followup or Phase V).

package providers

import (
	"errors"
	"fmt"
	"sort"
	"strings"
)

// AuthMode classifies how a provider authenticates requests.
type AuthMode string

const (
	AuthOAuth          AuthMode = "oauth"           // Claude MAX subscription via OAuth token
	AuthAPIKey         AuthMode = "api-key"         // Standard bearer/x-api-key header
	AuthServiceAccount AuthMode = "service-account" // Google service-account credential
)

// Provider describes one LLM provider's endpoint + auth conventions.
type Provider struct {
	Name             string   // canonical provider id (matches hub.AgentModelConfig.Provider)
	DefaultEndpoint  string   // empty for oauth/no-base-url paths (Claude direct)
	AuthMode         AuthMode // primary auth pattern
	AnthropicCompat  bool     // true if endpoint accepts Anthropic Messages API format
	DefaultModelName string   // suggested model when not user-specified
	Notes            string   // human-readable context (vendor doc URL, capability notes)
}

// registry holds the known-provider catalog. Single-source-of-truth
// for provider validation + endpoint defaults.
//
// Source: vendor product pages + Anthropic-compat endpoint docs as of
// 2026-05-09. Update as providers add/change Anthropic-compat support.
var registry = map[string]Provider{
	"anthropic": {
		Name:             "anthropic",
		DefaultEndpoint:  "", // direct via Claude Code; OAuth-driven
		AuthMode:         AuthOAuth,
		AnthropicCompat:  true,
		DefaultModelName: "claude-default",
		Notes:            "Claude MAX-subscription via Claude Code OAuth (R43 PATTERN-2 sanctioned subprocess)",
	},
	"deepseek": {
		Name:             "deepseek",
		DefaultEndpoint:  "https://api.deepseek.com/anthropic",
		AuthMode:         AuthAPIKey,
		AnthropicCompat:  true,
		DefaultModelName: "deepseek-v4-pro",
		Notes:            "DeepSeek-V4-Pro via Anthropic-compatible endpoint (capability-parity TESTS 1-7 validated)",
	},
	"openai": {
		Name:             "openai",
		DefaultEndpoint:  "https://api.openai.com/v1", // OpenAI native; not Anthropic-compat
		AuthMode:         AuthAPIKey,
		AnthropicCompat:  false,
		DefaultModelName: "gpt-4-turbo",
		Notes:            "OpenAI native API; requires adapter (deferred — T-8.6-followup)",
	},
	"google-vertex": {
		Name:             "google-vertex",
		DefaultEndpoint:  "https://us-central1-aiplatform.googleapis.com/v1",
		AuthMode:         AuthServiceAccount,
		AnthropicCompat:  false,
		DefaultModelName: "gemini-1.5-pro",
		Notes:            "Google Vertex AI; service-account auth; native API not Anthropic-compat",
	},
	"azure": {
		Name:             "azure",
		DefaultEndpoint:  "", // tenant-specific; user-supplied
		AuthMode:         AuthAPIKey,
		AnthropicCompat:  false,
		DefaultModelName: "gpt-4-turbo",
		Notes:            "Azure-hosted OpenAI; tenant-specific endpoint; user supplies BaseURL",
	},
	"mistral": {
		Name:             "mistral",
		DefaultEndpoint:  "https://api.mistral.ai/v1",
		AuthMode:         AuthAPIKey,
		AnthropicCompat:  false,
		DefaultModelName: "mistral-large-latest",
		Notes:            "Mistral AI native API; not Anthropic-compat",
	},
	"cohere": {
		Name:             "cohere",
		DefaultEndpoint:  "https://api.cohere.ai/v1",
		AuthMode:         AuthAPIKey,
		AnthropicCompat:  false,
		DefaultModelName: "command-r-plus",
		Notes:            "Cohere native API; not Anthropic-compat",
	},
	"together-ai": {
		Name:             "together-ai",
		DefaultEndpoint:  "https://api.together.xyz/v1",
		AuthMode:         AuthAPIKey,
		AnthropicCompat:  false,
		DefaultModelName: "meta-llama/Llama-3-70b-chat-hf",
		Notes:            "Together AI hosted open models; OpenAI-compatible API",
	},
}

// Lookup returns the Provider for the given canonical name. Returns
// ErrUnknownProvider when not in registry.
func Lookup(name string) (Provider, error) {
	p, ok := registry[name]
	if !ok {
		return Provider{}, fmt.Errorf("%w: %q", ErrUnknownProvider, name)
	}
	return p, nil
}

// IsKnown returns true when name is in the registry.
func IsKnown(name string) bool {
	_, ok := registry[name]
	return ok
}

// All returns the full provider catalog sorted by name (deterministic
// for UI rendering + tests).
func All() []Provider {
	out := make([]Provider, 0, len(registry))
	for _, p := range registry {
		out = append(out, p)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Name < out[j].Name })
	return out
}

// AnthropicCompatProviders returns providers that accept the Anthropic
// Messages API format (env-var swap pattern compatible).
func AnthropicCompatProviders() []Provider {
	all := All()
	out := make([]Provider, 0, len(all))
	for _, p := range all {
		if p.AnthropicCompat {
			out = append(out, p)
		}
	}
	return out
}

// ErrUnknownProvider is returned by Lookup + Validate when the provider
// name is not in the registry.
var ErrUnknownProvider = errors.New("unknown provider")

// Validate checks an agent-model-config against the provider registry.
// Takes primitive args to keep this package foundational (no hub import
// → no circular-import risk per Rain msg 17265 pre-fire design-flag).
//
// Returns nil for valid configs + descriptive error for known issues:
//
//   - unknown provider name
//   - missing required base_url for non-Anthropic-compat providers
//   - auth_secret_ref scheme mismatch with provider AuthMode
//
// Caller-side (hub.SetAgentModelConfig + webui settings handler + bot-hq
// config CLI) invoke this with cfg.Provider / cfg.ModelName / cfg.BaseURL
// / cfg.AuthSecretRef to surface validation errors pre-persistence.
func Validate(provider, modelName, baseURL, authSecretRef string) error {
	if provider == "" {
		return errors.New("provider is required")
	}
	p, err := Lookup(provider)
	if err != nil {
		return err
	}
	if !p.AnthropicCompat && baseURL == "" && p.DefaultEndpoint == "" {
		return fmt.Errorf("provider %q is not Anthropic-compat AND has no default endpoint; base_url required", provider)
	}
	if !validAuthSchemeForMode(authSecretRef, p.AuthMode) {
		return fmt.Errorf("auth_secret_ref %q scheme does not match provider %q auth-mode %q", authSecretRef, p.Name, p.AuthMode)
	}
	_ = modelName // accepted for future model-validation; currently unrestricted
	return nil
}

// validAuthSchemeForMode returns true when the auth_secret_ref scheme
// (oauth: / env: / file: / sa:) matches the provider's expected AuthMode.
//
// Permissive: env: is allowed for all modes (env-var indirection is
// always valid). oauth: only for AuthOAuth. sa: only for AuthServiceAccount.
func validAuthSchemeForMode(ref string, mode AuthMode) bool {
	if ref == "" {
		return false
	}
	scheme := strings.SplitN(ref, ":", 2)[0]
	switch scheme {
	case "env", "file":
		return true
	case "oauth":
		return mode == AuthOAuth
	case "sa":
		return mode == AuthServiceAccount
	default:
		return false
	}
}
