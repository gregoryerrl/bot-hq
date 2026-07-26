//! Per-provider capability metadata for the native loop.
//!
//! `AgentConfig` / `Model` already carry `provider` / `model_name` / `base_url`
//! / `auth_token` — everything needed to *reach* an endpoint. What they do not
//! carry is what the endpoint can *do*: how big the context window is, how the
//! credential is presented, where the messages endpoint lives. That is this
//! table.
//!
//! **It lives in code, not the `models` table or `policy.yaml`, on purpose.**
//! `models` rows are user-authored and a user should not be hand-entering a
//! context window; `policy.yaml` is about gates, not capabilities.

/// How a provider expects the credential to be presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthStyle {
    /// First-party Anthropic: `x-api-key: <token>` + `anthropic-version`.
    XApiKey,
    /// Anthropic-compatible gateways: `Authorization: Bearer <token>`.
    /// This is also what claude-code's `ANTHROPIC_AUTH_TOKEN` produces, which
    /// is how bot-hq's gateway agents authenticate today (`spawn.rs:1017`).
    Bearer,
}

/// What the native loop needs to know about a provider beyond its URL.
#[derive(Debug, Clone)]
pub struct ProviderProfile {
    /// Total context window in tokens. **Always `None` here — see below.**
    ///
    /// `ContextUsage` is only constructed when this is `Some`, so an unknown
    /// window renders as a gap in the UI rather than a guessed percentage — the
    /// same contract the claude-code path already honours
    /// (`AgentEvent::TurnComplete::context`).
    ///
    /// **A context window is a property of a MODEL, not a provider**, so this
    /// table structurally cannot know it: one provider serves several models
    /// with different windows, and a value keyed on the provider string would be
    /// confidently wrong for every model it didn't come from. The field stays
    /// here because the *shape* is right — a window belongs on the profile — but
    /// it is only ever populated from something that actually knows: a
    /// per-model value the user supplies, or direct measurement. Never from a
    /// figure quoted in a doc.
    pub context_window: Option<u64>,
    /// Default output-token budget when the caller doesn't override it. On
    /// Claude Opus 5 thinking and response text share this budget, so a
    /// `max_tokens` stop usually means "raise it", not "the answer was long".
    pub default_max_tokens: u32,
    /// Path appended to the agent's `base_url` (or to the first-party API root).
    pub messages_path: &'static str,
    pub auth: AuthStyle,
}

/// First-party Anthropic API root, used when an agent has no `base_url`.
pub const ANTHROPIC_API_ROOT: &str = "https://api.anthropic.com";

/// `anthropic-version` header value. Matches the spike.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

impl ProviderProfile {
    /// Resolve the profile for a `provider` string as stored on `AgentConfig` /
    /// `Model`. Matching is case-insensitive; an unrecognised provider falls
    /// back to [`Self::generic_gateway`], which is deliberately conservative:
    /// Bearer auth, unknown window.
    pub fn for_provider(provider: &str) -> Self {
        match provider.trim().to_ascii_lowercase().as_str() {
            // The one arm grounded in something this repo actually ran:
            // `examples/native_loop.rs` drives the first-party API with
            // `x-api-key` + `anthropic-version`.
            "anthropic" => Self {
                context_window: None,
                default_max_tokens: 16_000,
                messages_path: "/v1/messages",
                auth: AuthStyle::XApiKey,
            },
            // Everything else, including every named gateway, falls through.
            // A per-provider arm would only be worth adding for a fact this
            // repo can verify — and a context window is not one of those, since
            // it varies per model.
            _ => Self::generic_gateway(),
        }
    }

    /// Default for an Anthropic-compatible gateway.
    ///
    /// `Bearer` is an **assumption**, not a verified fact: bot-hq's CLI path
    /// hands gateways `ANTHROPIC_AUTH_TOKEN` (as against `ANTHROPIC_API_KEY`,
    /// which is the `x-api-key` spelling) and what claude-code does with it
    /// internally is not observable from this repo. First live contact settles
    /// it immediately — a wrong auth header is a 401 on turn one, not a subtle
    /// failure — so this is a cheap assumption to hold, unlike a context window
    /// that would silently render a wrong percentage forever.
    pub fn generic_gateway() -> Self {
        Self {
            context_window: None,
            default_max_tokens: 8_000,
            messages_path: "/v1/messages",
            auth: AuthStyle::Bearer,
        }
    }

    /// Full messages endpoint for an agent, given its configured `base_url`
    /// (`None` → first-party Anthropic). A trailing slash on `base_url` is
    /// tolerated so a user pasting `https://host/` from a provider's docs
    /// doesn't produce a `//v1/messages` 404.
    pub fn messages_url(&self, base_url: Option<&str>) -> String {
        let root = base_url
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .unwrap_or(ANTHROPIC_API_ROOT)
            .trim_end_matches('/');
        format!("{root}{}", self.messages_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_uses_x_api_key() {
        assert_eq!(
            ProviderProfile::for_provider("anthropic").auth,
            AuthStyle::XApiKey
        );
    }

    #[test]
    fn provider_match_is_case_insensitive_and_trims() {
        assert_eq!(
            ProviderProfile::for_provider("  AnThRoPiC ").auth,
            AuthStyle::XApiKey
        );
    }

    #[test]
    fn unknown_provider_falls_back_to_the_gateway_default() {
        let p = ProviderProfile::for_provider("some-new-gateway");
        assert_eq!(p.auth, AuthStyle::Bearer);
    }

    #[test]
    fn no_provider_ships_a_hardcoded_context_window() {
        // Regression guard. A window is a per-MODEL fact; a value keyed on the
        // provider string is confidently wrong for every model it didn't come
        // from, and the meter would render that wrongness as a precise
        // percentage. If a window is ever populated it must come from the user
        // or from measurement — never from a figure quoted in a doc.
        for provider in [
            "anthropic",
            "deepseek",
            "moonshot",
            "zhipu",
            "dashscope",
            "minimax",
            "",
            "anything-else",
        ] {
            assert_eq!(
                ProviderProfile::for_provider(provider).context_window,
                None,
                "provider {provider:?} must not declare a context window"
            );
        }
    }

    #[test]
    fn messages_url_defaults_to_first_party() {
        let p = ProviderProfile::for_provider("anthropic");
        assert_eq!(
            p.messages_url(None),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn messages_url_tolerates_a_trailing_slash_on_base_url() {
        let p = ProviderProfile::generic_gateway();
        assert_eq!(
            p.messages_url(Some("https://gateway.example/anthropic/")),
            "https://gateway.example/anthropic/v1/messages"
        );
    }

    #[test]
    fn messages_url_ignores_a_blank_base_url() {
        let p = ProviderProfile::for_provider("anthropic");
        assert_eq!(
            p.messages_url(Some("   ")),
            "https://api.anthropic.com/v1/messages"
        );
    }
}
