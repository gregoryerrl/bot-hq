//! **Layer 2 of an agent's system prompt — generated, never authored.**
//!
//! The design (§2) splits a spawned agent's prompt into three layers: core
//! rules (layer 1, `general_rules`), capability-derived rules (layer 2, here),
//! and the role's free-text prose (layer 3, `roles.description_prompt`). Only
//! layer 3 is stored on the role — migration 0044 says why in the schema
//! itself: *"a role must not be able to author rules that contradict its own
//! capability set."*
//!
//! Two rc3 decisions are implemented here.
//!
//! **D3 — the denials are generated from the ABSENT capabilities**, not
//! deleted and not hand-written. Reading design §2 literally ("lacks it → the
//! text is absent rather than misleading") deletes the explicit refusals, and
//! an agent then discovers each boundary by making a call that fails.
//! Hand-writing them is the drift §2 exists to prevent — CL
//! `learnings-2026-08-04` records *"a prompt can order a call a guard
//! refuses"*, three times in one session. Generating BOTH directions from one
//! `CapabilitySet` means the prompt and the enforced set cannot disagree,
//! because they are the same data.
//!
//! **D4 — peer names come from the live roster.** "Rain (EYES) can file
//! BLOCKING findings on your work" is a roster fact, not a constant. A session
//! with different or more participants has to describe itself truthfully.
//!
//! **A new `Capability` variant cannot silently go unmentioned.** [`phrasing`]
//! matches exhaustively, so adding one fails to COMPILE until its permission
//! line, its denial line and its third-person label are all written.

use super::capability::{Capability, CapabilitySet};

/// How one capability is worded, in all three places layer 2 needs it.
///
/// Held together in one struct, produced by one exhaustive `match`, so the
/// three can never be written for different capabilities or drift apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Phrasing {
    /// Second person, listed under "You may". A phrase, not a sentence — the
    /// heading supplies the verb.
    pub grant: &'static str,
    /// Second person, listed under "You may not". Names the refused tool, and
    /// what to do instead when there is something else to do.
    pub deny: &'static str,
    /// Third person, used when describing a PEER's capability set.
    pub peer: &'static str,
}

/// The single source for every capability's prompt wording.
///
/// **This match is the compile-time guarantee.** It is exhaustive, so a new
/// [`Capability`] variant makes this file fail to compile until someone decides
/// what the agent is told when it holds the capability AND what it is told when
/// it does not. There is no arm that can absorb a new variant silently.
pub fn phrasing(cap: Capability) -> Phrasing {
    use Capability::*;
    match cap {
        ReadChannel => Phrasing {
            grant: "read the session channel — every message posted after your cursor is \
                    delivered to you",
            deny: "read the session channel — you are delivered only what is handed to you \
                   directly, so never reason about messages you did not receive",
            peer: "reads the channel",
        },
        PostChannel => Phrasing {
            grant: "post to the session channel — what you write on your turn reaches the \
                    other participants and the user",
            deny: "post to the session channel — nothing you write reaches the other \
                   participants",
            peer: "posts to the channel",
        },
        AskUser => Phrasing {
            grant: "ask the user a question with `ask_user_choice(question, options)` — it \
                    parks and returns at once; the pick arrives later, out of band",
            deny: "ask the user anything — `ask_user_choice` is refused for you; surface the \
                   question to a participant who holds it",
            peer: "asks the user questions",
        },
        ParkApproval => Phrasing {
            grant: "park a per-action approval with `request_approval(kind, action)` before a \
                    sensitive action",
            deny: "park an approval — `request_approval` is refused for you",
            peer: "parks approvals",
        },
        SupersedeQuestion => Phrasing {
            grant: "replace one of your own parked questions with `supersede_question`",
            deny: "supersede a parked question — `supersede_question` is refused for you",
            peer: "supersedes parked questions",
        },
        Halt => Phrasing {
            grant: "yield the session to the user with `halt(reason)` or \
                    `mark_awaiting_user(reason)`",
            deny: "halt the session — `halt` and `mark_awaiting_user` are refused for you; say \
                   you are blocked and let a participant who can yield do it",
            peer: "halts the session for the user",
        },
        DeclareWorking => Phrasing {
            grant: "declare background work with `declare_working(reason, expected_seconds)` \
                    so the idle watchdog holds instead of nudging",
            deny: "declare background work — `declare_working` is refused for you",
            peer: "declares background work",
        },
        CloseSession => Phrasing {
            grant: "close the session with `close_session`, once the user has approved the \
                    close",
            deny: "close the session — `close_session` is refused for you",
            peer: "closes the session",
        },
        FileFinding => Phrasing {
            grant: "file a review finding with `eyes_flag(severity, summary)` — a `blocking` \
                    one mechanically gates commit and push until it is dispositioned",
            deny: "file a review finding — `eyes_flag` is refused for you; raise the problem \
                   in the channel instead",
            peer: "files review findings",
        },
        ApproveFinding => Phrasing {
            grant: "approve a finding with `approve_finding`",
            deny: "approve a finding — `approve_finding` is refused for you",
            peer: "approves findings",
        },
        DispositionFinding => Phrasing {
            grant: "clear a blocking finding with `disposition_finding(finding_id, status, \
                    reason)` — the commit gate holds until you do",
            deny: "clear a blocking finding — `disposition_finding` is refused for you; one \
                   filed against your work stays until a participant who holds it resolves it",
            peer: "clears blocking findings",
        },
        OverrideReviewerBlock => Phrasing {
            grant: "override a reviewer's block with `override_reviewer_block`",
            deny: "override a reviewer's block — `override_reviewer_block` is refused for you",
            peer: "overrides a reviewer's block",
        },
        EditFiles => Phrasing {
            grant: "edit files — `Edit`, `Write` and the mutating `Bash` forms are yours",
            deny: "edit files — `Edit`, `Write`, `NotebookEdit` and every mutating `Bash` form \
                   are blocked for you; propose the change instead of making it",
            peer: "edits files",
        },
        RunBash => Phrasing {
            // Deliberately not "run any shell command". `RunBash` is one grant
            // with no read-only variant, while the spawn path can hand a
            // participant a `--disallowedTools` list that denies the mutating
            // forms (`build_rain_disallowed_tools`). Wording the grant as
            // unconditional would make this section — which claims to be the
            // authority — overclaim against a restriction that really applies.
            grant: "run shell commands with `Bash`, within whatever your spawn's tool denies \
                    and the Tool Gate allow",
            deny: "run shell commands — `Bash` is blocked for you",
            peer: "runs shell commands",
        },
        GatedBash => Phrasing {
            grant: "route a Tool-Gate-blocked command through `action_gate(command)` for the \
                    user's approval — it parks and returns at once",
            deny: "route a gated command — `action_gate` is refused for you, so a command the \
                   Tool Gate blocks is simply unavailable; never reword one to slip past the \
                   gate",
            peer: "routes gated commands for approval",
        },
        RunTerminal => Phrasing {
            grant: "type into the session's visible terminal with `terminal_exec(command)`",
            deny: "type into the session's visible terminal — `terminal_exec` is refused for \
                   you; read what ran there with `terminal_read`",
            peer: "drives the session terminal",
        },
        WriteContextLibrary => Phrasing {
            grant: "write to the Context Library with `cl_write_file` and \
                    `cl_register_folder_description`",
            deny: "write to the Context Library — `cl_write_file` and \
                   `cl_register_folder_description` are refused for you; surface the \
                   correction in the channel instead",
            peer: "writes to the Context Library",
        },
    }
}

/// One other participant, as the roster knows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerFact {
    /// `session_participants.display_name` — what the user named them.
    pub display_name: String,
    /// The role's `display_name` (e.g. `EYES`), when the participant row points
    /// at one.
    pub role: Option<String>,
    /// The peer's invite-time snapshot, so the section can say what to hand
    /// them without naming a tool they do not hold.
    pub capabilities: CapabilitySet,
}

/// Everything layer 2 renders from: one participant's own grants plus the rest
/// of the live roster.
///
/// Assembled at spawn from `session_participants` — never from an agent name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterFacts {
    pub display_name: String,
    pub role: Option<String>,
    pub capabilities: CapabilitySet,
    /// The other ENABLED participants, in turn order.
    pub peers: Vec<PeerFact>,
}

/// `**Name** (ROLE)`, or `**Name**` when the row points at no role.
fn titled(display_name: &str, role: Option<&str>) -> String {
    match role.map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => format!("**{display_name}** ({r})"),
        None => format!("**{display_name}**"),
    }
}

/// Render layer 2: the capability rules, then the live roster.
///
/// Emitted AFTER every user-editable layer (see `read_system_prompt`), because
/// ordering is the only thing that stops free-text prose from arguing with it —
/// and the preamble says so out loud, so a model resolving a contradiction has
/// a stated rule rather than a guess.
pub fn render(facts: &RosterFacts) -> String {
    let mut out = String::with_capacity(2048);

    out.push_str(
        "## Capabilities — generated from this session's grants\n\n\
         Generated at spawn from the capability set this session gave you: the permissions \
         below from what it CONTAINS, the refusals from what it does NOT. It is one set, so \
         this section and the tool gate cannot disagree. **This section is the authority on \
         WHICH capabilities you hold** — a role description cannot widen or narrow the set, \
         so where the text above claims otherwise, this list is what actually happens. It is \
         not a licence to ignore narrower limits stated above: where a rule above CONSTRAINS \
         how you use something you hold, that constraint still binds.\n\n",
    );

    let held: Vec<Capability> = Capability::ALL
        .into_iter()
        .filter(|c| facts.capabilities.contains(*c))
        .collect();
    let absent: Vec<Capability> = Capability::ALL
        .into_iter()
        .filter(|c| !facts.capabilities.contains(*c))
        .collect();

    out.push_str("**You may:**\n\n");
    if held.is_empty() {
        out.push_str("- nothing — this session granted you no capabilities at all.\n");
    }
    for cap in &held {
        out.push_str("- ");
        out.push_str(phrasing(*cap).grant);
        out.push_str(".\n");
    }

    out.push_str("\n**You may not** — these calls are refused, so do not attempt them:\n\n");
    if absent.is_empty() {
        out.push_str("- nothing is withheld — you hold every capability bot-hq defines.\n");
    }
    for cap in &absent {
        out.push_str("- ");
        out.push_str(phrasing(*cap).deny);
        out.push_str(".\n");
    }

    out.push_str("\n## Participants in this session\n\n");
    out.push_str("You are ");
    out.push_str(&titled(&facts.display_name, facts.role.as_deref()));
    out.push_str(". ");
    if facts.peers.is_empty() {
        out.push_str(
            "You are the only participant — there is no peer to hand work to, and no peer \
             will review it.\n",
        );
        return out;
    }
    out.push_str("Also in this session:\n\n");
    for peer in &facts.peers {
        // What the peer can do THAT YOU CANNOT is the actionable half: it is the
        // list of things to hand over rather than attempt. A full dump of the
        // peer's set would be longer and mostly restate your own.
        let extra: Vec<&'static str> = Capability::ALL
            .into_iter()
            .filter(|c| peer.capabilities.contains(*c) && !facts.capabilities.contains(*c))
            .map(|c| phrasing(c).peer)
            .collect();
        out.push_str("- ");
        out.push_str(&titled(&peer.display_name, peer.role.as_deref()));
        if extra.is_empty() {
            out.push_str(" — holds no capability you lack.\n");
        } else {
            out.push_str(" — holds capabilities you do not: ");
            out.push_str(&extra.join(", "));
            out.push_str(".\n");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(caps: &[&str]) -> RosterFacts {
        RosterFacts {
            display_name: "Brian".into(),
            role: Some("HANDS".into()),
            capabilities: CapabilitySet::from_slugs(caps),
            peers: Vec::new(),
        }
    }

    /// The D3 invariant, one direction: a capability that IS present is stated
    /// as a permission and NOT as a refusal.
    ///
    /// Runs over every variant rather than a sample, so the pair of assertions
    /// also proves the two phrasings are distinguishable — a capability whose
    /// grant and deny read the same would make both assertions unfalsifiable.
    #[test]
    fn a_held_capability_produces_its_permission_and_no_denial() {
        for cap in Capability::ALL {
            let rendered = render(&facts(&[cap.slug()]));
            let p = phrasing(cap);
            assert!(
                rendered.contains(&format!("- {}.\n", p.grant)),
                "{} is held but its permission line is missing",
                cap.slug()
            );
            assert!(
                !rendered.contains(&format!("- {}.\n", p.deny)),
                "{} is held and was ALSO denied — the prompt contradicts itself",
                cap.slug()
            );
        }
    }

    /// The other direction, and the whole of D3: an ABSENT capability is stated
    /// as a refusal rather than left silent. Design §2 read literally deletes
    /// this text; the decision was to generate it instead.
    #[test]
    fn an_absent_capability_produces_its_denial_and_no_permission() {
        for cap in Capability::ALL {
            // Everything EXCEPT `cap`, so only one line can move between runs.
            let others: Vec<&str> = Capability::ALL
                .into_iter()
                .filter(|c| *c != cap)
                .map(|c| c.slug())
                .collect();
            let rendered = render(&facts(&others));
            let p = phrasing(cap);
            assert!(
                rendered.contains(&format!("- {}.\n", p.deny)),
                "{} is absent but nothing tells the agent so",
                cap.slug()
            );
            assert!(
                !rendered.contains(&format!("- {}.\n", p.grant)),
                "{} is absent and was granted anyway",
                cap.slug()
            );
        }
    }

    /// Every capability is worded in BOTH directions, and the three phrasings
    /// are distinct.
    ///
    /// The compile-time half of this guarantee is [`phrasing`]'s exhaustive
    /// match — a new variant does not compile until all three are written. What
    /// this adds is that none of them is a placeholder or a copy of another.
    #[test]
    fn every_capability_is_worded_in_both_directions() {
        for cap in Capability::ALL {
            let p = phrasing(cap);
            for (what, text) in [("grant", p.grant), ("deny", p.deny), ("peer", p.peer)] {
                assert!(!text.trim().is_empty(), "{}'s {what} is empty", cap.slug());
            }
            assert_ne!(p.grant, p.deny, "{}'s grant and deny are identical", cap.slug());
        }
    }

    /// `Capability::ALL` is hand-maintained (Rust cannot enumerate an enum), so
    /// it is cross-checked against two sources that are maintained separately:
    /// the two seeded presets and the tool→capability map. A capability missing
    /// from `ALL` is rendered in NEITHER direction — invisible in the prompt
    /// while still enforced at the gate.
    #[test]
    fn all_covers_every_capability_the_presets_and_the_tool_map_name() {
        for cap in Capability::ALL {
            assert!(
                Capability::ALL.iter().filter(|c| **c == cap).count() == 1,
                "{} appears twice in ALL, so it would be rendered twice",
                cap.slug()
            );
        }
        // Every capability either seeded preset grants must be renderable.
        let hands = CapabilitySet::preset_hands();
        let eyes = CapabilitySet::preset_eyes();
        for slug in hands.to_slugs().into_iter().chain(eyes.to_slugs()) {
            let cap = Capability::parse(slug).expect("a preset slug must parse");
            assert!(
                Capability::ALL.contains(&cap),
                "{slug} is granted by a seeded preset but missing from Capability::ALL"
            );
        }
        // And every capability that gates a tool: absent from ALL, the agent is
        // never told the tool is refused.
        for tool in [
            "ask_user_choice",
            "mark_awaiting_user",
            "halt",
            "request_approval",
            "action_gate",
            "supersede_question",
            "disposition_finding",
            "override_reviewer_block",
            "declare_working",
            "terminal_exec",
            "eyes_flag",
            "approve_finding",
            "cl_write_file",
            "cl_register_folder_description",
            "close_session",
        ] {
            let cap = super::super::capability::required_for(tool)
                .unwrap_or_else(|| panic!("{tool} is expected to be gated"));
            assert!(
                Capability::ALL.contains(&cap),
                "{tool} is gated on {} but that is missing from Capability::ALL",
                cap.slug()
            );
        }
    }

    /// D4: the peer section is roster data. Rename the participant and the
    /// prompt renames — no constant to update, because there is no constant.
    #[test]
    fn peer_names_come_from_the_roster_not_from_a_constant() {
        let mut f = facts(&["read_channel", "post_channel", "edit_files"]);
        f.peers = vec![PeerFact {
            display_name: "Ripley".into(),
            role: Some("AUDITOR".into()),
            capabilities: CapabilitySet::from_slugs(&["read_channel", "file_finding"]),
        }];
        let rendered = render(&f);
        assert!(
            rendered.contains("- **Ripley** (AUDITOR) —"),
            "the peer's roster name and role did not reach the prompt:\n{rendered}"
        );
        assert!(
            rendered.contains("files review findings"),
            "the peer's capability delta is missing:\n{rendered}"
        );
        // Nothing hardcoded leaks in. "Rain" is the name D4 removes, and it is
        // the one a hand-written peer paragraph would still be carrying.
        assert!(!rendered.contains("Rain"), "a hardcoded peer name survived");
        // The participant's OWN roster identity is rendered the same way.
        assert!(rendered.contains("You are **Brian** (HANDS)."), "{rendered}");
    }

    /// The delta is what the PEER holds and YOU do not — the actionable half.
    /// A peer whose set is a subset of yours has nothing to hand over, and
    /// saying so beats listing capabilities you already hold.
    #[test]
    fn a_peer_delta_is_relative_to_your_own_set() {
        let mut f = facts(&["read_channel", "post_channel", "edit_files", "run_bash"]);
        f.peers = vec![PeerFact {
            display_name: "Echo".into(),
            role: None,
            capabilities: CapabilitySet::from_slugs(&["read_channel", "run_bash"]),
        }];
        let rendered = render(&f);
        assert!(
            rendered.contains("- **Echo** — holds no capability you lack.\n"),
            "a subset peer must not be listed as holding anything extra:\n{rendered}"
        );
        assert!(
            !rendered.contains("runs shell commands"),
            "a capability you already hold was listed as the peer's:\n{rendered}"
        );
    }

    /// A solo roster says so rather than describing an absent peer.
    #[test]
    fn a_solo_participant_is_told_there_is_no_peer() {
        let rendered = render(&facts(&["read_channel", "post_channel"]));
        assert!(
            rendered.contains("You are the only participant"),
            "a solo session must say so:\n{rendered}"
        );
        assert!(!rendered.contains("Also in this session"));
    }

    /// An observer — `read_channel` and nothing else — is a legal role, and its
    /// prompt has to be coherent rather than a wall with one line missing.
    #[test]
    fn an_empty_grant_set_still_renders_a_coherent_section() {
        let rendered = render(&facts(&[]));
        assert!(rendered
            .contains("- nothing — this session granted you no capabilities at all."));
        // Every capability is still stated as refused, so the agent knows the
        // shape of what it cannot do rather than nothing at all.
        for cap in Capability::ALL {
            assert!(
                rendered.contains(&format!("- {}.\n", phrasing(cap).deny)),
                "{} was neither granted nor refused",
                cap.slug()
            );
        }
    }
}
