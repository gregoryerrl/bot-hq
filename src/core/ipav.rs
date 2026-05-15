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
}
