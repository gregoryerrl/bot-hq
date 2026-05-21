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

    /// Whether peer-forward buffering uses the 1.5s window (I/P) or pure
    /// turn-based forwarding on `message_stop` (A/V).
    pub fn uses_buffered_interleave(&self) -> bool {
        matches!(self, IpavPhase::Investigate | IpavPhase::Plan)
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
    fn buffered_only_in_investigate_plan() {
        assert!(IpavPhase::Investigate.uses_buffered_interleave());
        assert!(IpavPhase::Plan.uses_buffered_interleave());
        assert!(!IpavPhase::Apply.uses_buffered_interleave());
        assert!(!IpavPhase::Verify.uses_buffered_interleave());
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
