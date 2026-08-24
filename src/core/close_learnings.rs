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

/// How long the ring gets to DEAL the epilogue turn before the close gives up
/// on it ever starting.
///
/// Separate from [`CLOSE_EPILOGUE_TIMEOUT`] because the two failures are not the
/// same event and must not wear the same label: the ring is stopped when the
/// epilogue asks (`decide` only runs it from `Idle`/`AwaitingUser`), so the deal
/// is a release plus a stdin write and takes about as long as a subprocess
/// notices its input — seconds at most. Spending the whole 180s budget on the
/// arming half would report "the ring never dealt it" as
/// [`Outcome::TimedOut`], which reads as a slow agent and sends the next
/// diagnosis into the wrong half of the system.
pub const CLOSE_EPILOGUE_ARM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// The two halt texts the PUMP mints when a writer stops being askable — a
/// provider limit, or the transient-retry ladder exhausting into a
/// back-to-back error streak. Owned HERE, by the consumer, and interpolated
/// at both mint sites, so the detector and the text cannot drift apart.
pub const PROVIDER_LIMIT_HALT_PREFIX: &str = "⚠ Provider limit:";
pub const ERROR_STREAK_HALT_MARKER: &str = "turns are failing back-to-back";

/// Is the session's standing halt one of the pump's "the agent cannot
/// answer" declarations? Round 13, from `s-58f9a790`: the close epilogue
/// dealt its learnings turn to a credit-dead writer, got the provider-limit
/// error back, classified it as a DECLINE, and recorded "the session had
/// nothing worth recording" over a session that had read fifteen dossiers —
/// a false archive claim minted by asking a dead model.
pub fn writer_unwell(halt_reason: Option<&str>) -> bool {
    halt_reason.is_some_and(|r| {
        r.starts_with(PROVIDER_LIMIT_HALT_PREFIX) || r.contains(ERROR_STREAK_HALT_MARKER)
    })
}

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
    /// The session's standing halt says the writer cannot answer — a provider
    /// limit or a back-to-back error streak ([`writer_unwell`]). Asking it
    /// anyway returns the error text, which the outcome classifier reads as a
    /// decline and records as "nothing worth recording" — a false claim the
    /// next session orients from (round 13, `s-58f9a790`). Unlike
    /// `SkipNoWriter` this POSTS a row: the user deserves the honest why.
    SkipWriterUnwell,
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
    writer_unwell: bool,
    path: ClosePath,
) -> Epilogue {
    if !matches!(
        activity,
        SessionActivity::Idle | SessionActivity::AwaitingUser
    ) {
        return Epilogue::SkipBusy;
    }
    // Before the capability check: an unwell writer typically halted the
    // session (AwaitingUser passes the gate above), still holds the
    // capability, and still cannot be asked.
    if writer_unwell {
        return Epilogue::SkipWriterUnwell;
    }
    if !any_writer {
        return Epilogue::SkipNoWriter;
    }
    // `cl_written` gates BOTH paths — it is evidence of a write, whoever is
    // closing. `close_nudged` gates only the agent's own path, and the
    // difference is the point: it records that A3b soft-gated the AGENT's
    // `close_session` call, which says nothing about a close the USER started.
    // Reading it on the user's path suppressed exactly the case D15 exists to
    // cover ("the USER's Close button ... kills every subprocess without anyone
    // being asked"), because an agent that called the tool once and never
    // retried had already set the flag.
    let already = cl_written || (close_nudged && matches!(path, ClosePath::Agent));
    if already {
        return Epilogue::SkipAlreadyHandled;
    }
    Epilogue::Run
}

/// Who started this close.
///
/// Not cosmetic: A3b's write-the-delta nudge runs on the agent's `close_session`
/// tool and nowhere else, so its flag is only meaningful about that path. The
/// enum is what stops [`decide`] having to guess — it had no way to tell, and
/// read one path's bookkeeping as the other's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosePath {
    /// The agent's own `close_session` MCP tool, via the control-event worker.
    /// A3b already had its turn here.
    Agent,
    /// The user's Close button, or a plugin closing on the user's behalf —
    /// neither passes through A3b. **This is the path D15 was written for.**
    User,
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
    /// Another close is already running the epilogue for this session: do
    /// nothing and let it finish.
    ///
    /// **This arm used to be `TearDownNow`, and that killed the thing the other
    /// close had just started.** The two entry points (the UI's Close button and
    /// the agent's `close_session` tool) can both decide `Run`; the loser of the
    /// claim then tore down — SIGKILLing the agents mid-broadcast, so the
    /// epilogue turn produced no outcome row at all. Which is a race the second
    /// path reaches routinely, not rarely: the epilogue's own agent calling
    /// `close_session` on the turn it was just given IS the second close.
    ///
    /// Nothing leaks by waiting. The winner closed the session row before its
    /// turn (that is what `RunEpilogueFirst` means), so the session is already
    /// out of the UI, and the winner tears down when the turn ends.
    JoinInFlight,
}

/// [`decide`]'s answer, whether this call won the epilogue claim, and whether
/// another close is ALREADY mid-epilogue for the session.
///
/// `in_flight` is the fact that decides first (round 11): a second close for a
/// session whose learnings turn is running joins it — a double-clicked Close,
/// or the epilogue's own agent calling `close_session` on the turn it was just
/// given, which decides `SkipBusy` (the turn is in flight) or
/// `SkipAlreadyHandled` (A3b nudged it and it wrote), never `Run`. Keying the
/// join on `(Run, !claimed)` alone made the arm unreachable for exactly that
/// case, so the agent's own close tore the epilogue down mid-turn — the defect
/// the arm was written for.
pub fn plan(decision: Epilogue, claimed: bool, in_flight: bool) -> ClosePlan {
    if in_flight {
        return ClosePlan::JoinInFlight;
    }
    match (decision, claimed) {
        (Epilogue::Run, true) => ClosePlan::RunEpilogueFirst,
        // Wanted the epilogue and did not get the claim ⇒ somebody else holds
        // it (a race between two `Run`s inside the claim window).
        (Epilogue::Run, false) => ClosePlan::JoinInFlight,
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
    /// The prompt was delivered and the ring never dealt the turn within
    /// [`CLOSE_EPILOGUE_ARM_TIMEOUT`] — nobody was ever marked busy, so there was
    /// no turn to wait for.
    ///
    /// Distinct from [`Self::TimedOut`] on purpose. Both mean "no answer", and
    /// they point at opposite halves of the system: a timeout is an agent taking
    /// too long, this is the ring not running. bot-hq's recurring defect is
    /// ending states that are indistinguishable from a different one — a capped
    /// halt that read as a yield, a skipped epilogue that logged nothing, a
    /// yielded session that read as working — so a fourth "no answer" arm that
    /// collapsed into an existing label would be the same mistake again.
    NeverStarted,
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
        Outcome::NeverStarted => format!(
            "[System: close-out learnings — the turn was never dealt within {}s (the ring \
             did not run it). Nothing was written and the close proceeded.]",
            CLOSE_EPILOGUE_ARM_TIMEOUT.as_secs()
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

    /// Round 13 (`s-58f9a790`): an unwell writer is skipped BEFORE anyone
    /// asks it anything — whatever the flags say — and only from the stopped
    /// states (a busy close stays a busy close).
    #[test]
    fn an_unwell_writer_is_never_asked() {
        use SessionActivity::*;
        assert_eq!(
            decide(AwaitingUser, true, false, false, true, ClosePath::User),
            Epilogue::SkipWriterUnwell
        );
        assert_eq!(
            decide(Idle, true, false, false, true, ClosePath::Agent),
            Epilogue::SkipWriterUnwell
        );
        // Busy still outranks it: a force-close mid-turn is the user's stop.
        assert_eq!(
            decide(Busy, true, false, false, true, ClosePath::User),
            Epilogue::SkipBusy
        );
        // And a healthy writer is unaffected.
        assert_eq!(
            decide(AwaitingUser, true, false, false, false, ClosePath::User),
            Epilogue::Run
        );
    }

    #[test]
    fn writer_unwell_matches_the_pump_halts_and_nothing_else() {
        assert!(writer_unwell(Some(
            "⚠ Provider limit: \"You're out of usage credits.\" — the agent can't continue"
        )));
        assert!(writer_unwell(Some(
            "⚠ hands's turns are failing back-to-back (last error: \"x\")."
        )));
        assert!(!writer_unwell(Some("All done — the tray holds your next picks.")));
        assert!(!writer_unwell(None));
    }

    #[test]
    fn an_idle_session_with_a_writer_that_has_not_written_gets_the_turn() {
        assert_eq!(
            decide(SessionActivity::Idle, true, false, false, false, ClosePath::User),
            Epilogue::Run
        );
        // The dominant real case: the agent asked "Close session?" and the user
        // closed from the UI, so the session is parked on the user, not idle.
        assert_eq!(
            decide(SessionActivity::AwaitingUser, true, false, false, false, ClosePath::User),
            Epilogue::Run
        );
    }

    #[test]
    fn a_roster_with_no_context_library_writer_is_skipped_silently() {
        // D15: "a capability answer, not a special case" — a review-only
        // session has nobody who COULD write, so there is nothing to report.
        assert_eq!(
            decide(SessionActivity::Idle, false, false, false, false, ClosePath::User),
            Epilogue::SkipNoWriter
        );
    }

    #[test]
    fn a_session_that_already_wrote_or_was_already_asked_is_not_asked_again() {
        assert_eq!(
            decide(SessionActivity::Idle, true, true, false, false, ClosePath::User),
            Epilogue::SkipAlreadyHandled
        );
        // `close_nudged` is the agent's own close having been soft-gated for a
        // delta (jsonrpc A3b). On THAT path it declined; re-asking is how a
        // correct decline becomes filler.
        assert_eq!(
            decide(SessionActivity::Idle, true, false, true, false, ClosePath::Agent),
            Epilogue::SkipAlreadyHandled
        );
    }

    #[test]
    fn the_agents_nudge_does_not_suppress_the_users_close() {
        // The leak this pins, found 2026-08-13 with zero epilogues recorded in
        // any session ever. A3b runs on the agent's `close_session` tool ONLY,
        // so its flag says nothing about a close the USER started — and D15's
        // stated scope is precisely "the USER's Close button reaches
        // AppState::close_session directly and kills every subprocess without
        // anyone being asked". An agent that called the tool once, was nudged
        // and never retried left the flag set; the user's Close then read it as
        // already-handled.
        assert_eq!(
            decide(SessionActivity::Idle, true, false, true, false, ClosePath::User),
            Epilogue::Run,
            "the user's close is what D15 is FOR; the agent's nudge is not its business"
        );
        // And the flag still binds the path it belongs to, so this is a
        // narrowing rather than a removal.
        assert_eq!(
            decide(SessionActivity::Idle, true, false, true, false, ClosePath::Agent),
            Epilogue::SkipAlreadyHandled
        );
    }

    #[test]
    fn a_real_write_settles_both_paths() {
        // `cl_written` is evidence, not bookkeeping: the delta is on disk
        // whoever closed. Nothing to narrow here, and pinning it is what stops
        // the path split above being over-applied to the flag that should
        // survive it.
        for path in [ClosePath::Agent, ClosePath::User] {
            assert_eq!(
                decide(SessionActivity::Idle, true, true, false, false, path),
                Epilogue::SkipAlreadyHandled,
                "{path:?}: a session that wrote is not asked again"
            );
        }
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
                decide(activity, true, false, false, false, ClosePath::User),
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
            decide(SessionActivity::Busy, false, true, true, false, ClosePath::Agent),
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
        assert_eq!(plan(Epilogue::Run, true, false), ClosePlan::RunEpilogueFirst);
        // **A second close for a session already mid-epilogue waits.** It used
        // to tear down — "finish the job" — which SIGKILLed the agents in the
        // middle of the turn the first close had just started, so the epilogue
        // produced no outcome row at all. And the second close is not a rare
        // double-click: the epilogue's own agent calling `close_session` on the
        // turn it was given arrives here every time it happens. Nothing leaks by
        // waiting — the winner closed the row before the turn and tears down
        // after it.
        assert_eq!(plan(Epilogue::Run, false, false), ClosePlan::JoinInFlight);
        for skip in [
            Epilogue::SkipNoWriter,
            Epilogue::SkipAlreadyHandled,
            Epilogue::SkipBusy,
        ] {
            assert_eq!(plan(skip, false, false), ClosePlan::TearDownNow);
            // A claim can't be won without a Run decision, but if the call site
            // ever inverted the two, this is where it shows.
            assert_eq!(plan(skip, true, false), ClosePlan::TearDownNow);
        }
    }

    /// **The in-flight fact decides first** (round 11). The epilogue's own
    /// agent calling `close_session` on the turn it was just given decides
    /// `SkipBusy` (its turn is in flight) or `SkipAlreadyHandled` (A3b nudged
    /// it and it wrote) — never `Run` — so keying the join on `(Run, !claimed)`
    /// left that close, the case the arm names as routine, tearing the
    /// epilogue down mid-turn.
    #[test]
    fn any_close_during_an_in_flight_epilogue_joins_it() {
        for decision in [
            Epilogue::Run,
            Epilogue::SkipNoWriter,
            Epilogue::SkipAlreadyHandled,
            Epilogue::SkipBusy,
        ] {
            assert_eq!(plan(decision, false, true), ClosePlan::JoinInFlight, "{decision:?}");
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
