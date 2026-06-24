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

    /// Accept either single-letter chips (`I`/`P`/`A`/`V`) or full names
    /// (`Investigate`/`Plan`/`Apply`/`Verify`). Used by both the external
    /// driver MCP (chips) and the internal agent-callable advance_phase
    /// (full names — matches what agents see in `[PHASE: …]` envelopes).
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "I" | "Investigate" => IpavPhase::Investigate,
            "P" | "Plan" => IpavPhase::Plan,
            "A" | "Apply" => IpavPhase::Apply,
            "V" | "Verify" => IpavPhase::Verify,
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
    pub phase_log: Vec<(IpavPhase, String)>, // (phase, timestamp ISO)
}

impl Default for IpavState {
    fn default() -> Self {
        Self {
            current_phase: IpavPhase::Investigate,
            phase_log: Vec::new(),
        }
    }
}

impl IpavState {
    pub fn advance(&mut self, target: IpavPhase, timestamp: String) {
        self.current_phase = target;
        self.phase_log.push((target, timestamp));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_records_history() {
        let mut s = IpavState::default();
        s.advance(IpavPhase::Plan, "t1".into());
        s.advance(IpavPhase::Apply, "t2".into());
        assert_eq!(s.current_phase, IpavPhase::Apply);
        assert_eq!(s.phase_log.len(), 2);
        assert_eq!(s.phase_log[0].0, IpavPhase::Plan);
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
        assert_eq!(IpavPhase::parse("apply"), None, "case-sensitive");
        assert_eq!(IpavPhase::parse("Coffee"), None);
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
