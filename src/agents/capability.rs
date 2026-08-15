//! Capabilities — what a participant may do, as DATA rather than as an
//! agent-name comparison.
//!
//! Batch B2 of the session-focused redesign. Today authorization is
//! `caller.agent != "brian"` against three hardcoded lists
//! (`signaling/jsonrpc.rs`); here it becomes a set carried by the participant,
//! snapshotted from its role at invite time.
//!
//! Three rules from the design, encoded here rather than in prose:
//! 1. **Grants only.** There are no "prohibitions" — a deny-list is derived
//!    from what is absent, so the two can never contradict.
//! 2. **Dependencies are structural.** `GatedBash` without `RunBash` is
//!    incoherent; `validate()` refuses it rather than letting the UI ship a
//!    role that cannot work.
//! 3. **Session policy is the ceiling.** `effective = role ∩ session_policy`;
//!    a role may be more restricted than the session permits, never less.

use std::collections::BTreeSet;

/// One grant. `BTreeSet`-ordered so a serialized set is stable across writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    // ---- channel ----
    /// Read the session channel. A read-only role has this and nothing else.
    ReadChannel,
    /// Post to the session channel.
    PostChannel,
    // ---- user-facing ----
    AskUser,
    ParkApproval,
    SupersedeQuestion,
    Halt,
    CloseSession,
    // ---- review ----
    FileFinding,
    ApproveFinding,
    DispositionFinding,
    OverrideReviewerBlock,
    // ---- execution ----
    EditFiles,
    RunBash,
    /// Route a gated command for approval. Meaningless without `RunBash`:
    /// you cannot gate a command you are not allowed to run.
    GatedBash,
    RunTerminal,
    // ---- knowledge ----
    WriteContextLibrary,
}

impl Capability {
    /// Every capability, in the order the prompt lists them and the Roles tab
    /// renders its checklist — declaration order, which is also `Ord` order, so
    /// a `BTreeSet` walk agrees with this array, and which groups the 17 boxes
    /// into five short sections rather than one wall (see [`Capability::group`]).
    ///
    /// It exists so neither consumer carries a copy. A slug list hardcoded in
    /// TypeScript would drift the first time a capability is added here, and the
    /// drift is silent — the new grant simply never appears as a box, so no role
    /// can be given it.
    ///
    /// **What is and is not compile-enforced.** Rust cannot enumerate an enum
    /// without a macro, so this array is hand-maintained. What IS enforced by
    /// the compiler is the thing that matters: `capability_prompt::phrasing`
    /// matches exhaustively on `Capability`, so adding a variant fails to
    /// COMPILE until the author writes its permission line, its denial line and
    /// its third-person label — as do [`group`], [`label`] and [`description`]
    /// below. `all_lists_every_variant_exactly_once` and `capability_prompt`'s
    /// `all_covers_every_capability_the_presets_and_the_tool_map_name` then
    /// cross-check this array against three independent sources.
    pub const ALL: [Capability; 16] = {
        use Capability::*;
        [
            ReadChannel,
            PostChannel,
            AskUser,
            ParkApproval,
            SupersedeQuestion,
            Halt,
            CloseSession,
            FileFinding,
            ApproveFinding,
            DispositionFinding,
            OverrideReviewerBlock,
            EditFiles,
            RunBash,
            GatedBash,
            RunTerminal,
            WriteContextLibrary,
        ]
    };

    /// The section this capability is filed under in the Roles tab, matching
    /// the `---- … ----` bands the variants are declared in above.
    pub fn group(self) -> &'static str {
        use Capability::*;
        match self {
            ReadChannel | PostChannel => "Channel",
            AskUser | ParkApproval | SupersedeQuestion | Halt | CloseSession => {
                "User-facing"
            }
            FileFinding | ApproveFinding | DispositionFinding | OverrideReviewerBlock => "Review",
            EditFiles | RunBash | GatedBash | RunTerminal => "Execution",
            WriteContextLibrary => "Knowledge",
        }
    }

    /// Short human name for the checkbox.
    pub fn label(self) -> &'static str {
        use Capability::*;
        match self {
            ReadChannel => "Read the channel",
            PostChannel => "Post to the channel",
            AskUser => "Ask the user",
            ParkApproval => "Park an approval",
            SupersedeQuestion => "Supersede a question",
            Halt => "Halt",
            CloseSession => "Close the session",
            FileFinding => "File a finding",
            ApproveFinding => "Approve a finding",
            DispositionFinding => "Disposition a finding",
            OverrideReviewerBlock => "Override a reviewer block",
            EditFiles => "Edit files",
            RunBash => "Run Bash",
            GatedBash => "Route a gated command",
            RunTerminal => "Run the terminal",
            WriteContextLibrary => "Write the Context Library",
        }
    }

    /// One line on what the grant actually buys, phrased from what
    /// [`required_for`] gates and what the tool does — not from role names.
    pub fn description(self) -> &'static str {
        use Capability::*;
        match self {
            ReadChannel => "See the session's messages. A read-only role has this and nothing else.",
            PostChannel => "Speak in the session channel.",
            AskUser => "Put a multiple-choice question to the user (`ask_user_choice`).",
            ParkApproval => "Park a request for the user to approve (`request_approval`).",
            SupersedeQuestion => "Replace a question already waiting on the user.",
            Halt => "Stop and hand the session back to the user.",
            CloseSession => "End the session.",
            FileFinding => "Raise a finding against work — the reviewer's block.",
            ApproveFinding => "Sign a finding off.",
            DispositionFinding => "Resolve a finding filed against this participant's work.",
            OverrideReviewerBlock => "Proceed past a blocking finding.",
            EditFiles => "Write to files in the workspace.",
            RunBash => "Run shell commands.",
            GatedBash => "Send a gated command to the user for approval — needs `run_bash`.",
            RunTerminal => "Drive the session's terminal (`terminal_exec`).",
            WriteContextLibrary => "Add to or edit the Context Library.",
        }
    }

    pub fn slug(self) -> &'static str {
        use Capability::*;
        match self {
            ReadChannel => "read_channel",
            PostChannel => "post_channel",
            AskUser => "ask_user",
            ParkApproval => "park_approval",
            SupersedeQuestion => "supersede_question",
            Halt => "halt",
            CloseSession => "close_session",
            FileFinding => "file_finding",
            ApproveFinding => "approve_finding",
            DispositionFinding => "disposition_finding",
            OverrideReviewerBlock => "override_reviewer_block",
            EditFiles => "edit_files",
            RunBash => "run_bash",
            GatedBash => "gated_bash",
            RunTerminal => "run_terminal",
            WriteContextLibrary => "write_context_library",
        }
    }

    pub fn parse(slug: &str) -> Option<Self> {
        use Capability::*;
        Some(match slug {
            "read_channel" => ReadChannel,
            "post_channel" => PostChannel,
            "ask_user" => AskUser,
            "park_approval" => ParkApproval,
            "supersede_question" => SupersedeQuestion,
            "halt" => Halt,
            "close_session" => CloseSession,
            "file_finding" => FileFinding,
            "approve_finding" => ApproveFinding,
            "disposition_finding" => DispositionFinding,
            "override_reviewer_block" => OverrideReviewerBlock,
            "edit_files" => EditFiles,
            "run_bash" => RunBash,
            "gated_bash" => GatedBash,
            "run_terminal" => RunTerminal,
            "write_context_library" => WriteContextLibrary,
            _ => return None,
        })
    }

    /// Capabilities this one is incoherent without.
    pub fn requires(self) -> &'static [Capability] {
        use Capability::*;
        match self {
            // Gating presupposes running.
            GatedBash => &[RunBash],
            // Closing a session you cannot see.
            CloseSession => &[ReadChannel],
            // Every user-facing verb needs a voice in the channel.
            AskUser | ParkApproval | SupersedeQuestion | Halt => &[PostChannel],
            _ => &[],
        }
    }
}

/// Which capability a tool call requires. `None` = ungated (both roles today).
///
/// This is the single mapping that replaces `HANDS_ONLY_TOOLS`,
/// `EYES_ONLY_TOOLS` and `CL_MUTATE_TOOLS`. The parity oracle
/// (`signaling::parity`) is what proves the replacement is faithful.
pub fn required_for(tool: &str) -> Option<Capability> {
    use Capability::*;
    Some(match tool {
        "ask_user_choice" => AskUser,
        "mark_awaiting_user" | "halt" => Halt,
        "request_approval" => ParkApproval,
        "action_gate" => GatedBash,
        // NOT `withdraw_question`, deliberately (A4): gating it behind this
        // capability is a real permission change — the parity oracle flags it as
        // a divergence from the name gate rc3 replaced, and says in as many
        // words that such a change is the USER's call. What the audit actually
        // reported (any participant clearing any other's question out of the
        // user's tray) is closed by scoping the withdrawal to the row's asker in
        // `bridge::tray::withdraw_question`, which changes no capability. The
        // capability half is parked for the user.
        "supersede_question" => SupersedeQuestion,
        "disposition_finding" => DispositionFinding,
        "override_reviewer_block" => OverrideReviewerBlock,
        "terminal_exec" => RunTerminal,
        "eyes_flag" => FileFinding,
        "approve_finding" => ApproveFinding,
        "cl_write_file" | "cl_register_folder_description" => WriteContextLibrary,
        "close_session" => CloseSession,
        _ => return None,
    })
}

/// A participant's grants.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilitySet(BTreeSet<Capability>);

impl CapabilitySet {
    pub fn from_slugs(slugs: &[&str]) -> Self {
        Self(slugs.iter().filter_map(|s| Capability::parse(s)).collect())
    }

    /// Decode the stored `capabilities` column — a JSON array of slugs.
    ///
    /// `None` means the column is not a JSON array of strings at all. It is a
    /// separate answer from `Some(empty)` because the two must degrade
    /// differently: an empty array is a real configuration (a role that watches
    /// and nothing more), while
    /// a malformed column is a read failure, and rendering a prompt that denies
    /// everything on a read failure would tell an agent it cannot do things it
    /// demonstrably can.
    ///
    /// A slug that parses as JSON but names no capability is DROPPED, not an
    /// error — 0044 seeds `hands` with `route_gated_command`, which is not a
    /// variant. Grants are the only thing stored, so an unrecognised one is a
    /// grant nothing can honour.
    pub fn from_json(raw: &str) -> Option<Self> {
        let slugs: Vec<String> = serde_json::from_str(raw).ok()?;
        Some(Self(
            slugs.iter().filter_map(|s| Capability::parse(s)).collect(),
        ))
    }

    pub fn to_slugs(&self) -> Vec<&'static str> {
        self.0.iter().map(|c| c.slug()).collect()
    }

    pub fn contains(&self, cap: Capability) -> bool {
        self.0.contains(&cap)
    }

    /// May a participant with this set call `tool`? An ungated tool is allowed
    /// to everyone — over-gating is as much a parity break as under-gating.
    pub fn allows_tool(&self, tool: &str) -> bool {
        match required_for(tool) {
            Some(cap) => self.contains(cap),
            None => true,
        }
    }

    /// Structural coherence. Returns every unmet dependency rather than the
    /// first, so the Roles tab can show them all at once.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        for cap in &self.0 {
            for dep in cap.requires() {
                if !self.contains(*dep) {
                    errs.push(format!(
                        "`{}` requires `{}`",
                        cap.slug(),
                        dep.slug()
                    ));
                }
            }
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs)
        }
    }

    /// Configurations that are legal but probably not what the user meant.
    /// ADVISORY — never blocks. The Roles tab renders these.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.contains(Capability::FileFinding) && self.contains(Capability::DispositionFinding) {
            out.push(
                "self-review: this role can file a blocking finding and clear its own — \
                 the commit gate has no teeth for it"
                    .to_string(),
            );
        }
        if !self.contains(Capability::PostChannel) && self.contains(Capability::ReadChannel) {
            // Not "observer" — rc3 D18 deleted the participation mode of that
            // name, and a warning that reuses a retired mode's word reads as
            // "you set this role to observer" when the user set no such thing.
            out.push("read-only: this role sees the session but can never respond".to_string());
        }
        let executes = self.contains(Capability::EditFiles) || self.contains(Capability::RunBash);
        if executes && !self.contains(Capability::AskUser) {
            out.push(
                "silent worker: this role can act but can never surface a question to the user"
                    .to_string(),
            );
        }
        out
    }

    /// Session policy is the ceiling: a role may be MORE restricted than the
    /// session allows, never less.
    pub fn intersect(&self, ceiling: &CapabilitySet) -> Self {
        Self(self.0.intersection(&ceiling.0).copied().collect())
    }

    /// The user's HANDS role as seeded by migration 0044. Must grant exactly
    /// today's HANDS-only tools plus execution — parity, not preference.
    pub fn preset_hands() -> Self {
        use Capability::*;
        Self(BTreeSet::from([
            ReadChannel,
            PostChannel,
            AskUser,
            ParkApproval,
            SupersedeQuestion,
            Halt,
            CloseSession,
            DispositionFinding,
            OverrideReviewerBlock,
            EditFiles,
            RunBash,
            GatedBash,
            RunTerminal,
            WriteContextLibrary,
        ]))
    }

    /// The user's EYES role: reviews, reads, runs read-only shell. No edit, no
    /// user-facing verbs — today's deny-list expressed as absence.
    pub fn preset_eyes() -> Self {
        use Capability::*;
        Self(BTreeSet::from([
            ReadChannel,
            PostChannel,
            FileFinding,
            ApproveFinding,
            RunBash,
        ]))
    }
}

/// What a gated decision site actually has to work with: either a participant's
/// decoded grants, or the fact that they could not be read.
///
/// # Why this is not `Option<CapabilitySet>`
///
/// `None` and `Some(empty)` would both spell "holds nothing", and they must not.
/// An empty set is a real configuration — a role deliberately granted nothing. An unreadable roster is a *read failure*, and the two want different
/// error text and different reasoning at the site.
///
/// # Degradation: gated decisions fail CLOSED, and say why
///
/// [`Self::Unreadable`] denies every gated tool and withholds every capability.
/// This is the OPPOSITE of the choice `resolve_roster_facts` makes for the
/// prompt layer (which renders no layer 2 at all rather than a prompt that
/// denies everything), and deliberately so — the two have inverted risk
/// profiles:
///
/// * The prompt is *advice*. Advice that wrongly says "you may do nothing"
///   strands an agent that demonstrably can, so silence is the safer failure.
/// * A gate is *enforcement*. Failing open silently means EYES quietly gains
///   HANDS's tools and nothing in the UI, the logs or the transcript says so —
///   the adversarial split the whole session rests on evaporates without a
///   symptom. Failing closed is loud: the refusal names the missing capability
///   and the reason the roster could not answer, so the failure is reported by
///   the agent on its next turn instead of being discovered later in a diff.
///
/// The blast radius is bounded by design: an UNGATED tool stays allowed even
/// when unreadable (see [`Self::allows_tool`]), so a roster failure cannot deny
/// a call the name gate never gated either. What an unreadable roster costs is
/// exactly the set of tools that were always gated.
///
/// **Known interaction, not hypothetical.** `ensure_session_roster` is called
/// pre-spawn and its failure is warned-and-continued
/// (`core::session::ensure_session_started`), so a session CAN reach a spawn
/// with no participant rows. Before capability enforcement that session ran
/// normally on the name gate; now its gated tools are refused. That is the
/// price of fail-closed, taken knowingly: the alternative is a session that
/// looks fine while nothing is enforced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedCapabilities {
    /// The participant row was found and its `capabilities` column decoded.
    Known(CapabilitySet),
    /// The roster could not answer for this caller. `reason` is a short,
    /// stable phrase quoted into the refusal so the failure is diagnosable
    /// from the agent's transcript alone.
    Unreadable { reason: &'static str },
}

impl ResolvedCapabilities {
    /// May this caller call `tool`?
    ///
    /// An ungated tool is allowed unconditionally — including when the roster is
    /// unreadable. Over-gating is as much a parity break as under-gating, and a
    /// read failure is not a reason to start gating calls that were never gated.
    pub fn allows_tool(&self, tool: &str) -> bool {
        match required_for(tool) {
            None => true,
            Some(cap) => self.grants(cap),
        }
    }

    /// Does this caller hold `cap`? `false` when the roster is unreadable.
    pub fn grants(&self, cap: Capability) -> bool {
        match self {
            Self::Known(set) => set.contains(cap),
            Self::Unreadable { .. } => false,
        }
    }

    /// The read failure's reason, for the refusal text. `None` when resolved.
    pub fn unreadable_reason(&self) -> Option<&'static str> {
        match self {
            Self::Known(_) => None,
            Self::Unreadable { reason } => Some(reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Today's three gate lists, copied from `signaling/jsonrpc.rs`. The point
    /// of this test is that the capability model reproduces them EXACTLY — the
    /// same contract the parity oracle asserts at the dispatch layer.
    const HANDS_ONLY: &[&str] = &[
        "ask_user_choice", "mark_awaiting_user", "request_approval", "action_gate",
        "supersede_question", "disposition_finding", "override_reviewer_block",
    ];
    const EYES_ONLY: &[&str] = &["eyes_flag", "approve_finding"];
    const CL_MUTATE: &[&str] = &["cl_write_file", "cl_register_folder_description"];

    #[test]
    fn hands_preset_reproduces_todays_hands_only_boundary() {
        let hands = CapabilitySet::preset_hands();
        for tool in HANDS_ONLY {
            assert!(hands.allows_tool(tool), "HANDS must still allow {tool}");
        }
        for tool in CL_MUTATE {
            assert!(hands.allows_tool(tool), "HANDS must still allow {tool}");
        }
        for tool in EYES_ONLY {
            assert!(!hands.allows_tool(tool), "HANDS must not gain {tool}");
        }
    }

    #[test]
    fn eyes_preset_reproduces_todays_eyes_boundary() {
        let eyes = CapabilitySet::preset_eyes();
        for tool in EYES_ONLY {
            assert!(eyes.allows_tool(tool), "EYES must still allow {tool}");
        }
        for tool in HANDS_ONLY {
            assert!(!eyes.allows_tool(tool), "EYES must not gain {tool}");
        }
        for tool in CL_MUTATE {
            assert!(!eyes.allows_tool(tool), "EYES must not gain {tool}");
        }
    }

    #[test]
    fn ungated_tools_are_allowed_to_everyone() {
        // Over-gating is a parity break too, and a quieter one.
        let empty = CapabilitySet::default();
        for tool in [
            "session_doc_write",
            "cl_retrieve",
            "check_open_findings",
            "peer_ack",
            // Declining a turn is available to anything that can hold one, so
            // an EMPTY capability set must still admit it. A role that could
            // not pass would be pushed back onto the two endings the pass
            // exists to replace — a false done vote, or filler.
            "pass_turn",
        ] {
            assert!(empty.allows_tool(tool), "{tool} is ungated today");
        }
    }

    #[test]
    fn both_presets_are_structurally_valid() {
        assert!(CapabilitySet::preset_hands().validate().is_ok());
        assert!(CapabilitySet::preset_eyes().validate().is_ok());
    }

    #[test]
    fn dependencies_are_enforced() {
        // Gating a command you cannot run.
        let e = CapabilitySet::from_slugs(&["gated_bash"]).validate().unwrap_err();
        assert!(e.iter().any(|m| m.contains("`gated_bash` requires `run_bash`")), "{e:?}");
        // Closing a session you cannot see.
        let e = CapabilitySet::from_slugs(&["close_session"]).validate().unwrap_err();
        assert!(e.iter().any(|m| m.contains("requires `read_channel`")), "{e:?}");
        // Every unmet dependency is reported, not just the first.
        let e = CapabilitySet::from_slugs(&["gated_bash", "close_session"])
            .validate()
            .unwrap_err();
        assert_eq!(e.len(), 2, "both dependencies must be reported: {e:?}");
    }

    #[test]
    fn session_policy_is_the_ceiling() {
        let role = CapabilitySet::from_slugs(&["run_bash", "gated_bash", "edit_files"]);
        let ceiling = CapabilitySet::from_slugs(&["run_bash", "gated_bash"]);
        let effective = role.intersect(&ceiling);
        assert!(!effective.contains(Capability::EditFiles), "the ceiling must win");
        assert!(effective.contains(Capability::RunBash));
        // Intersection is commutative — a role cannot gain from the ceiling.
        assert_eq!(effective, ceiling.intersect(&role));
    }

    #[test]
    fn warnings_flag_self_review_and_silent_workers() {
        let self_review = CapabilitySet::from_slugs(&["file_finding", "disposition_finding"]);
        assert!(
            self_review.warnings().iter().any(|w| w.contains("self-review")),
            "a role that files and clears its own findings must be flagged"
        );
        // HANDS holds DispositionFinding but NOT FileFinding → not self-review.
        assert!(
            !CapabilitySet::preset_hands().warnings().iter().any(|w| w.contains("self-review")),
            "the HANDS preset must not trip the self-review warning"
        );
        let silent = CapabilitySet::from_slugs(&["read_channel", "post_channel", "run_bash"]);
        assert!(silent.warnings().iter().any(|w| w.contains("silent worker")));
    }

    #[test]
    fn close_session_is_the_one_intended_boundary_change() {
        // Today `close_session` is UNGATED — EYES can close a session, which is
        // the live gap found in CL issues #5 ("Rain once called close_session
        // while Brian was one ToolSearch away from writing CL learnings —
        // nothing was persisted"). The design fixes it as a capability.
        //
        // This is therefore a DELIBERATE difference from today's behaviour, and
        // the only one in B2. Asserting it here means it can never be mistaken
        // for a parity regression during B6 — and if someone later decides EYES
        // should keep the ability, this test is where that conversation starts.
        assert_eq!(required_for("close_session"), Some(Capability::CloseSession));
        assert!(CapabilitySet::preset_hands().allows_tool("close_session"));
        assert!(
            !CapabilitySet::preset_eyes().allows_tool("close_session"),
            "closing the session is the one capability EYES loses relative to today"
        );
        // Note it is deliberately absent from the parity oracle's UNGATED list
        // (`signaling::parity`), so the two artifacts do not contradict.
    }

    #[test]
    fn all_lists_every_variant_exactly_once() {
        // The Roles tab renders ONE checkbox per entry of `ALL`, so a variant
        // missing from it is a grant no role can ever be given — and nothing
        // errors, because the checklist simply has one fewer box.
        //
        // The match below is exhaustive, so a NEW variant fails to compile
        // here until it is handled; the count assertion then fails until it is
        // added to `ALL` as well.
        use Capability::*;
        let mut seen = BTreeSet::new();
        for cap in Capability::ALL {
            match cap {
                ReadChannel | PostChannel | AskUser | ParkApproval | SupersedeQuestion | Halt
                | CloseSession | FileFinding | ApproveFinding
                | DispositionFinding | OverrideReviewerBlock | EditFiles | RunBash | GatedBash
                | RunTerminal | WriteContextLibrary => {}
            }
            assert!(seen.insert(cap), "{} is listed twice in ALL", cap.slug());
        }
        assert_eq!(
            seen.len(),
            16,
            "a capability is missing from `Capability::ALL` (16 since \
             declare_working retired, 2026-08-15: working = holding a turn)"
        );
    }

    #[test]
    fn every_listed_capability_carries_the_labels_the_roles_tab_renders() {
        // `list_capabilities` is the tab's ONLY source for the checklist, so a
        // blank label or description renders as an unnamed, unexplained box the
        // user has to guess at — and guessing wrong here hands a role a grant.
        for cap in Capability::ALL {
            assert!(!cap.label().is_empty(), "{} has no label", cap.slug());
            assert!(
                !cap.description().is_empty(),
                "{} has no description",
                cap.slug()
            );
            assert!(!cap.group().is_empty(), "{} has no group", cap.slug());
            // A label that IS the slug means the row was added to `ALL` and the
            // metadata matches were left to fall through to a placeholder.
            assert_ne!(cap.label(), cap.slug(), "{} is unlabelled", cap.slug());
        }
    }

    #[test]
    fn exactly_four_capabilities_gate_no_tool() {
        // `capability_prompt`'s module doc publishes a table of WHERE each
        // capability is enforced, and tells the agent to treat a refusal as
        // binding precisely because a few are carried by the prompt alone. That
        // is a factual claim about this file, so it is pinned here rather than
        // left to rot the first time a tool mapping is added.
        //
        // A capability with no tool in `required_for` is not enforced by the
        // tool gate. Of the four below, `edit_files` IS enforced — at spawn, by
        // the child's permission posture and its MCP servers
        // (`spawn::build_command`, `core::session::user_mcp_servers_for_agent`).
        // The other three have no enforcement site at all today.
        let registry = crate::signaling::protocol::tool_descriptors();
        let unmapped: Vec<&'static str> = Capability::ALL
            .into_iter()
            // No tool in the live registry maps to this capability, so no call
            // the gate sees can ever depend on holding it.
            .filter(|c| !registry.iter().any(|d| required_for(d.name) == Some(*c)))
            .map(|c| c.slug())
            .collect();
        assert_eq!(
            unmapped,
            vec!["read_channel", "post_channel", "edit_files", "run_bash"],
            "the set of capabilities the TOOL GATE cannot enforce changed — \
             update the enforcement table in `capability_prompt`'s module doc \
             and the Roles tab's capability hint before changing this"
        );
    }

    #[test]
    fn an_unreadable_set_denies_gated_tools_and_leaves_ungated_ones_alone() {
        // The degradation contract, at the type rather than through dispatch.
        // The gate short-circuits ungated tools before it ever asks
        // (`jsonrpc::capability_gated`), so nothing reaching `dispatch` can
        // exercise the `None` arm below — and an arm no test reaches is an arm
        // that can be changed without anything going red. This is that test.
        let broken = ResolvedCapabilities::Unreadable {
            reason: "the session roster could not be read",
        };
        for tool in HANDS_ONLY.iter().chain(EYES_ONLY).chain(CL_MUTATE) {
            assert!(
                !broken.allows_tool(tool),
                "{tool} is gated — an unreadable set must refuse it"
            );
        }
        for tool in ["session_doc_write", "cl_retrieve", "peer_ack", "pass_turn"] {
            assert!(
                broken.allows_tool(tool),
                "{tool} was never gated — an unreadable set must not start gating it"
            );
        }
        // No capability is held, and the reason survives for the refusal text.
        for cap in Capability::ALL {
            assert!(!broken.grants(cap), "{} must not be granted", cap.slug());
        }
        assert_eq!(
            broken.unreadable_reason(),
            Some("the session roster could not be read")
        );

        // A resolved-but-EMPTY set is a different thing: a real configuration
        // with no grants, and it must not claim to be a read failure.
        let granted_nothing = ResolvedCapabilities::Known(CapabilitySet::default());
        assert_eq!(granted_nothing.unreadable_reason(), None);
        assert!(!granted_nothing.allows_tool("ask_user_choice"));
        assert!(granted_nothing.allows_tool("pass_turn"));
    }

    #[test]
    fn slugs_round_trip() {
        // The set is persisted as JSON slugs; a parse/render asymmetry would
        // silently drop a grant on read.
        for set in [CapabilitySet::preset_hands(), CapabilitySet::preset_eyes()] {
            let slugs = set.to_slugs();
            assert_eq!(CapabilitySet::from_slugs(&slugs), set);
        }
    }
}
