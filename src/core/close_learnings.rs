//! The close-out learnings epilogue (rc3 **D15**).
//!
//! D15's second half: *"On close — the session writes its own learnings, if it
//! has any worth writing."* The gap it closes is narrow and real — the agent's
//! own `close_session` MCP call is already soft-gated with a write-the-delta
//! nudge (`jsonrpc.rs`, A3b), but the USER's Close button reaches
//! [`AppState::close_session`](crate::core::AppState::close_session) directly
//! and kills every subprocess without anyone being asked. That is D15's *"a
//! session that ends before anyone writes anything down."*
//!
//! Everything decidable lives in [`decide`] — one function, four arms, callable
//! from a test. The live half ([`AppState::run_close_epilogue`]) is a broadcast,
//! a bounded wait and a row; it holds no policy of its own. That split is the
//! CL's "test the WIRE, not the halves" rule applied up front: the interesting
//! part is which sessions get an epilogue, and it is not spread across the
//! close path.
//!
//! ## What D15 constrains, and where each constraint lands
//!
//! | D15 | here |
//! |---|---|
//! | close must not block | the caller marks the session closed and returns; this runs detached |
//! | no `write_context_library` holder → skipped **silently** | [`Epilogue::SkipNoWriter`], which posts no row |
//! | an empty-handed session writes **nothing** | [`CLOSE_LEARNINGS_PROMPT`] makes silence the expected outcome |
//! | a failed write must not look like a decline | [`Outcome`] has four arms and [`outcome_notice`] gives each its own row |

use crate::core::activity::SessionActivity;

/// How long the epilogue's turn may run before the close stops waiting for it.
///
/// Sized for what the turn actually is — read one CL file, append ~5 lines,
/// write it back — not for a task. It is a ceiling, not a budget: the common
/// path is an agent that declines in seconds. The cost of overshooting is a
/// subprocess alive a little longer after the session left the UI; the cost of
/// undershooting is a half-written CL file, so it errs long.
pub const CLOSE_EPILOGUE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

/// Whether a closing session gets a learnings turn, and if not, why not.
///
/// The `Skip*` arms are distinct because they are not the same event: two of
/// them are the session having already done the thing, one is a capability
/// answer and one is the user closing mid-turn. Collapsing them to a bool is
/// what would make the close path's behaviour unexplainable later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Epilogue {
    /// Broadcast the prompt, wait for the turn, post the outcome row.
    Run,
    /// No participant holds `write_context_library`. D15: *"it is skipped
    /// silently — that is a capability answer, not a special case."* No row.
    SkipNoWriter,
    /// The session already wrote a CL delta this session, or the agent's own
    /// `close_session` was already nudged for one. Asking again is how a
    /// session that correctly declined gets talked into filler.
    SkipAlreadyHandled,
    /// The session was mid-turn. Closing then is the user saying stop (D15's
    /// own framing), and a broadcast into a live turn fights the sequencer.
    SkipBusy,
}

/// The entire decision, in one place.
///
/// Order matters and is not arbitrary: `SkipBusy` outranks the rest because a
/// user force-closing a working session has already said what they want, and
/// `SkipNoWriter` outranks `SkipAlreadyHandled` so a session with no writer is
/// reported as the capability answer it is rather than as bookkeeping.
pub fn decide(
    activity: SessionActivity,
    any_writer: bool,
    cl_written: bool,
    close_nudged: bool,
) -> Epilogue {
    if !matches!(
        activity,
        SessionActivity::Idle | SessionActivity::AwaitingUser
    ) {
        return Epilogue::SkipBusy;
    }
    if !any_writer {
        return Epilogue::SkipNoWriter;
    }
    if cl_written || close_nudged {
        return Epilogue::SkipAlreadyHandled;
    }
    Epilogue::Run
}

/// What the close path does, as data.
///
/// This exists so the epilogue arm cannot be deleted quietly. The CL's
/// "test the WIRE, not the halves" rule names the better fix as *making the
/// wrong version fail to COMPILE*: `close_session` matches exhaustively on
/// this, so dropping [`Self::RunEpilogueFirst`] is a non-exhaustive match, not
/// a green suite with an inert feature. Five separate defects of exactly that
/// shape shipped in one week before this rule was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosePlan {
    /// Kill and clean up now; this call closes the session row too.
    TearDownNow,
    /// Close the row and release the UI first, run the learnings turn, then
    /// tear down.
    RunEpilogueFirst,
}

/// [`decide`]'s answer plus whether this call actually won the epilogue claim.
///
/// `claimed` is false when another close is already mid-epilogue for the same
/// session — a double-clicked Close, or the epilogue's own agent calling the
/// `close_session` tool on the turn it was just given. A second close means
/// finish the job, so it tears down.
pub fn plan(decision: Epilogue, claimed: bool) -> ClosePlan {
    match (decision, claimed) {
        (Epilogue::Run, true) => ClosePlan::RunEpilogueFirst,
        _ => ClosePlan::TearDownNow,
    }
}

/// The close-time instruction.
///
/// **Phrased to permit, and expect, silence** — D15 is explicit that this is
/// the whole difficulty: *"An agent with nothing to say, prompted at close to
/// write its learnings, does not return empty — it produces plausible filler,"*
/// and that filler lands in the one layer whose purpose is that future sessions
/// orient from it instead of re-reading the code. So the prompt states the bar,
/// then states that clearing it is the exception. A version of this that says
/// "write what you learned" is a defect, not a wording preference.
///
/// A raw string, unlike `prompts.rs` / `general_rules.rs`, which are
/// `"\`-continued escaped literals where a bare `"` breaks the file.
pub const CLOSE_LEARNINGS_PROMPT: &str = r#"[System: this session is closing — this is your last turn, and no one is waiting on a reply.

If this session turned up something a FUTURE session would need and could not recover in seconds from the code or git history — a gotcha, a where-things-live pointer, a convention you had to infer — append it to the project's Context Library now with cl_write_file: read the file's current body first and write the FULL replacement, at most ~5 one-line entries.

Writing nothing is the expected outcome and needs no explanation. Most sessions learn nothing worth persisting, and an invented entry is worse than an empty one: the next session reads it as fact and builds on it. Do not summarise this session, do not restate what the code already says, and do not write a "nothing to report" note anywhere.

Whichever you choose, say nothing further — the session closes when your turn ends.]"#;

/// What the epilogue's turn came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A CL write landed during the turn.
    Wrote,
    /// The turn finished and wrote nothing. **This is a success**, and the row
    /// says so — an agent that had nothing worth persisting did the right
    /// thing.
    Declined,
    /// The turn was still running at [`CLOSE_EPILOGUE_TIMEOUT`].
    TimedOut,
    /// The prompt could not be delivered at all.
    Failed(String),
}

/// The one-line `system_notice` body for an outcome.
///
/// This is D15's remaining open item: *"a fire-and-forget write that FAILS
/// looks identical to one that correctly declined. Record the decision where
/// session state already lives — a visible row, as the capped halt posts one."*
/// Four outcomes, four bodies, so the two are never the same sentence. Sized
/// like the D7 round-cap notice, whose lane this shares.
pub fn outcome_notice(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Wrote => {
            "[System: close-out learnings — this session wrote to the Context Library \
             before closing.]"
                .to_string()
        }
        Outcome::Declined => {
            "[System: close-out learnings — the session had nothing worth recording, and \
             wrote nothing.]"
                .to_string()
        }
        Outcome::TimedOut => format!(
            "[System: close-out learnings — no answer within {}s. The close proceeded; \
             whether anything was written is unknown.]",
            CLOSE_EPILOGUE_TIMEOUT.as_secs()
        ),
        Outcome::Failed(e) => format!(
            "[System: close-out learnings — the session could not be asked ({e}). Nothing \
             was written and the close proceeded.]"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idle_session_with_a_writer_that_has_not_written_gets_the_turn() {
        assert_eq!(
            decide(SessionActivity::Idle, true, false, false),
            Epilogue::Run
        );
        // The dominant real case: the agent asked "Close session?" and the user
        // closed from the UI, so the session is parked on the user, not idle.
        assert_eq!(
            decide(SessionActivity::AwaitingUser, true, false, false),
            Epilogue::Run
        );
    }

    #[test]
    fn a_roster_with_no_context_library_writer_is_skipped_silently() {
        // D15: "a capability answer, not a special case" — a review-only
        // session has nobody who COULD write, so there is nothing to report.
        assert_eq!(
            decide(SessionActivity::Idle, false, false, false),
            Epilogue::SkipNoWriter
        );
    }

    #[test]
    fn a_session_that_already_wrote_or_was_already_asked_is_not_asked_again() {
        assert_eq!(
            decide(SessionActivity::Idle, true, true, false),
            Epilogue::SkipAlreadyHandled
        );
        // `close_nudged` is the agent's own close having been soft-gated for a
        // delta (jsonrpc A3b). It declined; re-asking is how a correct decline
        // becomes filler.
        assert_eq!(
            decide(SessionActivity::Idle, true, false, true),
            Epilogue::SkipAlreadyHandled
        );
    }

    #[test]
    fn closing_a_working_session_skips_the_turn_entirely() {
        // Busy, cancelling and paused are all "the user said stop".
        for activity in [
            SessionActivity::Busy,
            SessionActivity::Cancelling,
            SessionActivity::Paused,
        ] {
            assert_eq!(
                decide(activity, true, false, false),
                Epilogue::SkipBusy,
                "{activity:?} must not be interrupted for a learnings turn"
            );
        }
    }

    #[test]
    fn busy_outranks_every_other_reason_to_skip() {
        // Precedence, pinned: a mid-turn close is reported as the stop it is,
        // not as a capability or bookkeeping answer.
        assert_eq!(
            decide(SessionActivity::Busy, false, true, true),
            Epilogue::SkipBusy
        );
    }

    #[test]
    fn a_decline_and_a_failure_do_not_read_the_same() {
        // The D15 item this file exists to close. Four outcomes, four bodies.
        let bodies = [
            outcome_notice(&Outcome::Wrote),
            outcome_notice(&Outcome::Declined),
            outcome_notice(&Outcome::TimedOut),
            outcome_notice(&Outcome::Failed("bridge gone".into())),
        ];
        for (i, a) in bodies.iter().enumerate() {
            for b in bodies.iter().skip(i + 1) {
                assert_ne!(a, b, "every close-out outcome needs its own sentence");
            }
        }
        // And a decline must not read as a failure to a human skimming the log.
        let declined = outcome_notice(&Outcome::Declined);
        assert!(declined.contains("nothing worth recording"));
        assert!(!declined.to_lowercase().contains("fail"));
        assert!(outcome_notice(&Outcome::TimedOut).contains("180s"));
    }

    #[test]
    fn only_a_run_that_won_its_claim_gets_the_epilogue_arm() {
        assert_eq!(plan(Epilogue::Run, true), ClosePlan::RunEpilogueFirst);
        // A second close for a session already mid-epilogue: finish the job.
        assert_eq!(plan(Epilogue::Run, false), ClosePlan::TearDownNow);
        for skip in [
            Epilogue::SkipNoWriter,
            Epilogue::SkipAlreadyHandled,
            Epilogue::SkipBusy,
        ] {
            assert_eq!(plan(skip, false), ClosePlan::TearDownNow);
            // A claim can't be won without a Run decision, but if the call site
            // ever inverted the two, this is where it shows.
            assert_eq!(plan(skip, true), ClosePlan::TearDownNow);
        }
    }

    #[test]
    fn the_prompt_makes_writing_nothing_the_expected_outcome() {
        // D15 SETTLED: "the prompt must make 'write nothing' an expected,
        // blameless outcome rather than a failure to complete a task."
        assert!(CLOSE_LEARNINGS_PROMPT.contains("Writing nothing is the expected outcome"));
        assert!(CLOSE_LEARNINGS_PROMPT.contains("cl_write_file"));
        // ...and it must not ask for a stub in place of a write, which is the
        // other half of "an empty-handed session writes NOTHING".
        assert!(CLOSE_LEARNINGS_PROMPT.contains("nothing to report"));
        assert!(CLOSE_LEARNINGS_PROMPT.contains("do not write a"));
    }
}
