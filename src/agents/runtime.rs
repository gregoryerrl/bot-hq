//! Which agent CLI a participant runs through — the CONNECTOR, chosen per
//! model row (`models.runtime`, 0102) and recorded on the participant at its
//! first spawn (`session_participants.runtime`, pinned like the account dir).
//!
//! rc3 D9 made claude-code the only connector; the Codex connector (ChatGPT
//! Pro, 2026-10-21) amends that with the user's ratification, in core rather
//! than as a plugin because the plugin runtime cannot spawn processes.

use serde::{Deserialize, Serialize};

/// The connector a model row spawns through. The serialized form is the
/// column value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    /// claude-code: `claude -p --input-format stream-json …`.
    ClaudeCode,
    /// OpenAI's Codex CLI: `codex app-server` over stdio JSON-RPC.
    Codex,
}

impl Runtime {
    pub const ALL: [Runtime; 2] = [Runtime::ClaudeCode, Runtime::Codex];

    /// The column value.
    pub fn as_str(self) -> &'static str {
        match self {
            Runtime::ClaudeCode => "claude_code",
            Runtime::Codex => "codex",
        }
    }

    /// Exactly a column value, or `None` — a caller that must refuse (Save)
    /// uses this; nothing is trimmed or guessed.
    pub fn parse(value: &str) -> Option<Runtime> {
        Runtime::ALL.into_iter().find(|r| r.as_str() == value)
    }

    /// A column read: blank (a row from before 0102) and any value this build
    /// does not know both mean claude-code, the only connector such a row
    /// could have run. Save refuses the unknown values before they are stored.
    pub fn from_column(value: &str) -> Runtime {
        Runtime::parse(value).unwrap_or(Runtime::ClaudeCode)
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Runtime::ClaudeCode
    }
}

impl std::fmt::Display for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_round_trips_through_its_column_value() {
        assert_eq!(Runtime::parse("claude_code"), Some(Runtime::ClaudeCode));
        assert_eq!(Runtime::parse("codex"), Some(Runtime::Codex));
        assert_eq!(Runtime::ClaudeCode.as_str(), "claude_code");
        assert_eq!(Runtime::Codex.as_str(), "codex");
        for r in Runtime::ALL {
            assert_eq!(Runtime::parse(r.as_str()), Some(r));
        }
    }

    /// A value the column could hold from a newer or broken build is not
    /// guessed at: the caller decides whether that is a refusal (Save) or the
    /// safe default (a read). Blank is the pre-0102 row, which only ever ran
    /// claude-code.
    #[test]
    fn unknown_runtime_values_are_not_guessed() {
        assert_eq!(Runtime::parse("gemini"), None);
        assert_eq!(Runtime::parse(" codex "), None);
        assert_eq!(Runtime::parse(""), None);
        assert_eq!(Runtime::from_column(""), Runtime::ClaudeCode);
        assert_eq!(Runtime::from_column("codex"), Runtime::Codex);
        assert_eq!(Runtime::from_column("gemini"), Runtime::ClaudeCode);
    }
}
