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
    /// Total context window in tokens.
    ///
    /// **`None` means unknown, and unknown must stay visible.** `ContextUsage`
    /// is only constructed when this is `Some`, so an unknown window renders as
    /// a gap in the UI rather than a guessed percentage — the same contract the
    /// claude-code path already honours (`AgentEvent::TurnComplete::context`).
    /// Seeding this with a plausible-looking number would silently turn a known
    /// unknown into a wrong answer.
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
            "anthropic" => Self {
                // Deliberately unknown. The Messages API reports `usage` but
                // never the window, and Claude windows vary per model and per
                // beta tier — the CLI knew this number, we do not. B4 measures
                // it; until then the meter shows a gap instead of a guess.
                context_window: None,
                default_max_tokens: 16_000,
                messages_path: "/v1/messages",
                auth: AuthStyle::XApiKey,
            },
            // DeepSeek declares 200K. It has also been observed serving a
            // 238,155-token prompt against that declaration — either silent
            // truncation or a wrong declaration. Recorded here as *declared*,
            // and flagged: a native loop can measure request tokens directly,
            // which is what settles it (handoff Q4).
            "deepseek" => Self {
                context_window: Some(200_000),
                default_max_tokens: 8_000,
                messages_path: "/v1/messages",
                auth: AuthStyle::Bearer,
            },
            _ => Self::generic_gateway(),
        }
    }

    /// Conservative default for an Anthropic-compatible gateway we have no
    /// measured facts about.
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
    fn anthropic_uses_x_api_key_and_declares_no_window() {
        let p = ProviderProfile::for_provider("anthropic");
        assert_eq!(p.auth, AuthStyle::XApiKey);
        // A guessed window is worse than a visible gap — see the field doc.
        assert_eq!(p.context_window, None);
    }

    #[test]
    fn provider_match_is_case_insensitive_and_trims() {
        let p = ProviderProfile::for_provider("  DeepSeek ");
        assert_eq!(p.auth, AuthStyle::Bearer);
        assert_eq!(p.context_window, Some(200_000));
    }

    #[test]
    fn unknown_provider_falls_back_to_conservative_gateway() {
        let p = ProviderProfile::for_provider("some-new-gateway");
        assert_eq!(p.auth, AuthStyle::Bearer);
        assert_eq!(p.context_window, None);
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
        let p = ProviderProfile::for_provider("deepseek");
        assert_eq!(
            p.messages_url(Some("https://api.deepseek.com/anthropic/")),
            "https://api.deepseek.com/anthropic/v1/messages"
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
