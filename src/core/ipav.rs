//! IPAV (Investigate → Plan → Apply → Verify) in-memory phase state.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IpavPhase {
    Investigate,
    Plan,
    Apply,
    Verify,
}

impl IpavPhase {
    pub fn chip(&self) -> &'static str {
        match self {
            IpavPhase::Investigate => "I",
            IpavPhase::Plan => "P",
            IpavPhase::Apply => "A",
            IpavPhase::Verify => "V",
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            IpavPhase::Investigate => "Investigate",
            IpavPhase::Plan => "Plan",
            IpavPhase::Apply => "Apply",
            IpavPhase::Verify => "Verify",
        }
    }

    /// Canonical lowercase tag persisted as the `session_documents.phase` value
    /// and matched by the IPAV document tabs. The single source of truth for the
    /// session-doc phase vocabulary (`signaling/jsonrpc.rs::parse_optional_phase`
    /// normalizes any accepted casing/chip through here).
    pub fn tag(&self) -> &'static str {
        match self {
            IpavPhase::Investigate => "investigate",
            IpavPhase::Plan => "plan",
            IpavPhase::Apply => "apply",
            IpavPhase::Verify => "verify",
        }
    }

    /// Accept either single-letter chips (`I`/`P`/`A`/`V`) or full names
    /// (`Investigate`/`Plan`/`Apply`/`Verify`), case-INSENSITIVELY. Used by
    /// both the external driver MCP (chips) and the internal agent-callable
    /// advance_phase (full names — matches what agents see in `[PHASE: …]`
    /// envelopes). Case-insensitive so a lowercase `"apply"` arg (the form the
    /// session-doc tools accept) can't be valid for one phase tool and rejected
    /// by another — `signaling/jsonrpc.rs::parse_optional_phase` routes the
    /// session-doc phase arg through here too.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "i" | "investigate" => IpavPhase::Investigate,
            "p" | "plan" => IpavPhase::Plan,
            "a" | "apply" => IpavPhase::Apply,
            "v" | "verify" => IpavPhase::Verify,
            _ => return None,
        })
    }

    /// Canonical hint for INVALID_PARAMS error messages when `parse` rejects
    /// a target. Single source of truth so internal + external MCP dispatch
    /// can't drift apart on what they tell agents is acceptable.
    pub fn error_hint() -> &'static str {
        "I/P/A/V or Investigate/Plan/Apply/Verify"
    }

    /// Strong-framed notice fed to agents when this phase becomes active.
    /// Used by `AppState::advance_phase` so transitions carry weight instead
    /// of degrading into a passive "phase advanced to X" log line.
    pub fn transition_notice(&self) -> &'static str {
        match self {
            IpavPhase::Investigate => "[PHASE: Investigate] Gather facts only. No Edit, Write, or mutating Bash. Output understanding in chat.",
            IpavPhase::Plan => "[PHASE: Plan] Propose the approach in chat — name files, functions, expected diffs. No Edit/Write yet.",
            IpavPhase::Apply => "[PHASE: Apply] HANDS (Brian) executes Edit/Write/Bash. EYES (Rain) reviews — no writes from Rain. Apply output may be code or a document.",
            IpavPhase::Verify => "[PHASE: Verify] Run tests, type-check, re-read, or describe the manual check. Cite the output.",
        }
    }
}

/// Per-session IPAV runtime state. Held in `AppState` as `HashMap<SessionId, _>`.
#[derive(Debug, Clone)]
pub struct IpavState {
    pub current_phase: IpavPhase,
}

impl Default for IpavState {
    fn default() -> Self {
        Self {
            current_phase: IpavPhase::Investigate,
        }
    }
}

impl IpavState {
    pub fn advance(&mut self, target: IpavPhase) {
        self.current_phase = target;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_sets_current_phase() {
        let mut s = IpavState::default();
        s.advance(IpavPhase::Plan);
        assert_eq!(s.current_phase, IpavPhase::Plan);
        s.advance(IpavPhase::Apply);
        assert_eq!(s.current_phase, IpavPhase::Apply);
    }

    #[test]
    fn parse_accepts_chips_and_full_names() {
        assert_eq!(IpavPhase::parse("I"), Some(IpavPhase::Investigate));
        assert_eq!(IpavPhase::parse("Investigate"), Some(IpavPhase::Investigate));
        assert_eq!(IpavPhase::parse("P"), Some(IpavPhase::Plan));
        assert_eq!(IpavPhase::parse("Plan"), Some(IpavPhase::Plan));
        assert_eq!(IpavPhase::parse("A"), Some(IpavPhase::Apply));
        assert_eq!(IpavPhase::parse("Apply"), Some(IpavPhase::Apply));
        assert_eq!(IpavPhase::parse("V"), Some(IpavPhase::Verify));
        assert_eq!(IpavPhase::parse("Verify"), Some(IpavPhase::Verify));
        assert_eq!(IpavPhase::parse("Coffee"), None);
    }

    #[test]
    fn parse_is_case_insensitive() {
        // Lowercase full names (the form session_doc tools accept) now parse,
        // so advance_phase and the session-doc phase arg can't drift on case.
        assert_eq!(IpavPhase::parse("apply"), Some(IpavPhase::Apply));
        assert_eq!(IpavPhase::parse("investigate"), Some(IpavPhase::Investigate));
        assert_eq!(IpavPhase::parse("v"), Some(IpavPhase::Verify));
        assert_eq!(IpavPhase::parse("PLAN"), Some(IpavPhase::Plan));
    }

    #[test]
    fn transition_notice_starts_with_phase_envelope() {
        for phase in [
            IpavPhase::Investigate,
            IpavPhase::Plan,
            IpavPhase::Apply,
            IpavPhase::Verify,
        ] {
            let notice = phase.transition_notice();
            let expected_prefix = format!("[PHASE: {}]", phase.name());
            assert!(
                notice.starts_with(&expected_prefix),
                "{} notice missing prefix {expected_prefix:?}: {notice}",
                phase.name()
            );
        }
    }
}
