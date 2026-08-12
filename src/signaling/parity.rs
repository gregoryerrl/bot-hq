//! **Parity oracle** — the tool-authorization boundary as it behaves BEFORE the
//! session-focused redesign (batch B0.1 of
//! `docs/plans/2026-08-06-session-focused-redesign-implementation.md`).
//!
//! The redesign's hard constraint is the user's: *"this shouldn't change what
//! works today client-side."* That cannot be verified by being careful — it
//! needs assertions that pin today's behaviour and must still pass once
//! authorization moves from agent-name equality to capability sets. **A failure
//! here after the redesign is either a bug, or a decision that must be recorded
//! in the design doc's Constraint 0. It is never "just update the test".**
//!
//! These live in-crate rather than in `tests/` because `signaling::jsonrpc` is a
//! private module: an integration test cannot reach `dispatch` or
//! `CallerIdentity`. (The implementation plan originally said `tests/…`; this is
//! the correction.)
//!
//! They pin BEHAVIOUR, not the constant lists — asserting per-tool accept/reject
//! catches a tool being silently added to or dropped from a gate list, which
//! asserting the constants would not.

use super::jsonrpc::{dispatch, resolve_caller_capabilities, CallerIdentity};
use super::protocol::JsonRpcRequest;
use crate::signaling::SignalingBridge;
use serde_json::json;
use std::sync::Arc;

/// Every tool HANDS may call and EYES may not, as of 2026-08-06 (`jsonrpc.rs`
/// `HANDS_ONLY_TOOLS`).
const HANDS_ONLY: &[&str] = &[
    "ask_user_choice",
    "mark_awaiting_user",
    "request_approval",
    "action_gate",
    "supersede_question",
    "disposition_finding",
    "override_reviewer_block",
    "halt",
    "declare_working",
    "terminal_exec",
];

/// Every tool EYES may call and HANDS may not (`EYES_ONLY_TOOLS`).
const EYES_ONLY: &[&str] = &["eyes_flag", "approve_finding"];

/// CL-content mutations: HANDS writes, EYES reviews (`CL_MUTATE_TOOLS`).
const CL_MUTATE: &[&str] = &["cl_write_file", "cl_register_folder_description"];

/// Tools deliberately available to BOTH roles. Pinned so the rewrite cannot
/// over-gate: a capability model that accidentally restricts these would be a
/// silent behaviour change in the other direction.
const UNGATED: &[&str] = &[
    "session_doc_write",
    "session_doc_search",
    "cl_index_search",
    "cl_retrieve",
    "check_open_findings",
    "peer_ack",
    // Every participant that can hold a turn can decline one, so there is no
    // capability to gate this on — a role that could not pass would be forced
    // back to the two endings the pass exists to replace.
    "pass_turn",
    "advance_phase",
    "gate_status",
    // Ungated BEFORE the capability gate landed, and held that way by
    // `jsonrpc::PARITY_HOLD`. The capability model maps it to
    // `Capability::CloseSession`, which EYES is not seeded with, so routing it
    // through capabilities would newly refuse it for EYES — a behaviour change,
    // not a reframe. It stays on this list until that change is taken on its
    // own merits. See `the_parity_hold_is_exactly_the_known_divergence`.
    "close_session",
];

/// **The frozen oracle: the tool gate exactly as it read before rc3 wired
/// capabilities.** A transcription of `jsonrpc.rs` lines 208-224 as they stood
/// at commit `5dc5b53`:
///
/// ```text
/// if HANDS_ONLY_TOOLS.contains(&name) && caller.agent != "brian" { reject }
/// if CL_MUTATE_TOOLS.contains(&name) && caller.agent == "rain"   { reject }
/// if EYES_ONLY_TOOLS.contains(&name) && caller.agent != "rain"   { reject }
/// ```
///
/// **Do not "simplify" this to consult `CapabilitySet`.** Its entire value is
/// that it is an INDEPENDENT statement of the old behaviour. Re-deriving it from
/// the thing it exists to check would make every assertion below tautological,
/// which is the exact failure mode the reframe contract's rule 1 is written
/// against.
fn legacy_name_gate_admits(tool: &str, agent: &str) -> bool {
    !(HANDS_ONLY.contains(&tool) && agent != "brian")
        && !(CL_MUTATE.contains(&tool) && agent == "rain")
        && !(EYES_ONLY.contains(&tool) && agent != "rain")
}

/// A caller whose grants come from the DATABASE, resolved the way the HTTP
/// layer resolves them for a live RPC.
///
/// This is what makes the parity assertions bite. Handing `dispatch` a
/// hand-built `CapabilitySet` would prove the gate reads its argument; going
/// through `resolve_caller_capabilities` against a migrated database proves the
/// SEEDED roster produces the decisions the name gate produced.
async fn caller(bridge: &SignalingBridge, agent: &str) -> CallerIdentity {
    CallerIdentity {
        capabilities: resolve_caller_capabilities(bridge, "s-parity", agent).await,
        session_id: "s-parity".into(),
        agent: agent.into(),
    }
}

/// A bridge whose storage holds one session with the roster
/// `ensure_session_roster` seeds — i.e. what every real session gets.
async fn seeded_bridge() -> Arc<SignalingBridge> {
    use crate::storage::Storage;
    let bridge = SignalingBridge::new();
    let storage = Storage::memory().await.unwrap();
    bridge.set_storage(storage.clone()).await;
    storage.create_session("s-parity", "parity", None).await.unwrap();
    let seeded = storage.ensure_session_roster("s-parity").await.unwrap();
    assert_eq!(seeded, 2, "the parity roster must hold both participants");
    bridge
}

/// A `tools/call` carrying a superset of every gated tool's required args, so a
/// call reaches the authorization check rather than failing argument parsing.
/// The role gates run BEFORE the `match name` dispatch, so the extra keys are
/// inert for the rejection cases.
fn call(tool: &str) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: "tools/call".into(),
        params: Some(json!({
            "name": tool,
            "arguments": {
                "reason": "parity", "question": "?", "options": ["a", "b"],
                "kind": "push_gate", "action": "y", "command": "echo hi",
                "finding_id": 1, "status": "fixed", "summary": "s",
                "project": "bot-hq", "file_path": "notes.md", "content": "x",
                "slug": "d", "body": "b", "query": "q", "target": "Apply",
                "gate_id": "g", "choice_id": "c", "message": "m",
                "description": "d", "folder_path": "f", "severity": "advisory",
            }
        })),
    }
}

/// Did this dispatch come back as a GATE rejection (as opposed to succeeding, or
/// failing for an unrelated reason like absent storage)?
///
/// Keyed on the phrase every refusal `jsonrpc::gate_refusal` builds contains.
/// The three name-based messages it replaced are matched too — not because they
/// can still be produced, but so that a revert of the gate is caught by this
/// helper returning the right answer rather than by it silently reporting
/// "not rejected" for every tool, which would turn every assertion below green.
fn is_gate_rejection(rendered: &str) -> bool {
    rendered.contains("capability, which this session did not grant you")
        || rendered.contains("could not read your capability set")
        // pre-rc3 wording
        || rendered.contains("is reserved for the HANDS agent")
        || rendered.contains("is reserved for HANDS (brian)")
        || rendered.contains("is reserved for the EYES agent")
}

async fn rejected_by(bridge: &Arc<SignalingBridge>, tool: &str, agent: &str) -> bool {
    let who = caller(bridge, agent).await;
    let out = dispatch(call(tool), &who, bridge).await;
    let rendered = match out {
        Ok(Some(res)) => serde_json::to_value(&res).map(|v| v.to_string()).unwrap_or_default(),
        Ok(None) => String::new(),
        Err(e) => format!("{e:?}"),
    };
    is_gate_rejection(&rendered)
}

/// The gate's verdict for one (tool, agent), run through the real dispatch path
/// against the seeded roster.
async fn gate_admits(bridge: &Arc<SignalingBridge>, tool: &str, agent: &str) -> bool {
    !rejected_by(bridge, tool, agent).await
}

#[tokio::test]
async fn hands_only_tools_reject_eyes_and_admit_hands() {
    let bridge = seeded_bridge().await;
    for tool in HANDS_ONLY {
        assert!(
            rejected_by(&bridge, tool, "rain").await,
            "{tool} must reject EYES — it is HANDS-only today"
        );
        assert!(
            !rejected_by(&bridge, tool, "brian").await,
            "{tool} must NOT be role-rejected for HANDS"
        );
    }
}

#[tokio::test]
async fn eyes_only_tools_reject_hands_and_admit_eyes() {
    let bridge = seeded_bridge().await;
    for tool in EYES_ONLY {
        assert!(
            rejected_by(&bridge, tool, "brian").await,
            "{tool} must reject HANDS — EYES files findings, HANDS resolves them"
        );
        assert!(
            !rejected_by(&bridge, tool, "rain").await,
            "{tool} must NOT be role-rejected for EYES"
        );
    }
}

#[tokio::test]
async fn cl_mutating_tools_reject_eyes_and_admit_hands() {
    let bridge = seeded_bridge().await;
    for tool in CL_MUTATE {
        assert!(
            rejected_by(&bridge, tool, "rain").await,
            "{tool} must reject EYES — HANDS owns CL content authorship"
        );
        assert!(
            !rejected_by(&bridge, tool, "brian").await,
            "{tool} must NOT be role-rejected for HANDS"
        );
    }
}

#[tokio::test]
async fn ungated_tools_admit_both_roles() {
    // The other direction: the capability rewrite must not silently RESTRICT a
    // tool both roles can use today. Over-gating is as much a parity break as
    // under-gating, and is easier to ship unnoticed.
    let bridge = seeded_bridge().await;
    for tool in UNGATED {
        for agent in ["brian", "rain"] {
            assert!(
                !rejected_by(&bridge, tool, agent).await,
                "{tool} is ungated today — {agent} must not be role-rejected"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// B0.2 — the commit-gate contract
//
// The review contract in four assertions: what blocks, what does not, what
// clears it, and what happens when the reviewer is gone. B6 rewrites the gate's
// query from "Rain's findings" to "any participant holding FileFinding", and B4
// replaces the `rain_enabled` duo check with a roster lookup. These must survive
// both unchanged.
// ---------------------------------------------------------------------------

use crate::storage::{FindingSeverity, FindingStatus, Storage};

async fn bridge_with_session(sid: &str) -> std::sync::Arc<SignalingBridge> {
    let bridge = SignalingBridge::new();
    let storage = Storage::memory().await.unwrap();
    bridge.set_storage(storage.clone()).await;
    storage.create_session(sid, "parity", None).await.unwrap();
    bridge
}

#[tokio::test]
async fn a_blocking_finding_gates_the_commit_and_advisory_does_not() {
    let bridge = bridge_with_session("s1").await;
    assert_eq!(bridge.check_open_findings("s1").await.unwrap(), "ok");

    // Advisory: filed, visible, but NEVER gates. This is the distinction the
    // whole N-way generalisation rests on (design Q3) — if advisory ever starts
    // gating, "derived review authority" silently becomes "anyone can block".
    bridge
        .eyes_flag(
            "s1".into(),
            "rain".into(),
            FindingSeverity::Advisory,
            "style nit".into(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        bridge.check_open_findings("s1").await.unwrap(),
        "ok",
        "an advisory finding must not gate the commit"
    );

    // Blocking: gates, and names the finding so HANDS can act on it.
    let uid = bridge
        .eyes_flag(
            "s1".into(),
            "rain".into(),
            FindingSeverity::Blocking,
            "real bug".into(),
            Some("src/x.rs:1".into()),
        )
        .await
        .unwrap();
    let verdict = bridge.check_open_findings("s1").await.unwrap();
    assert!(verdict.starts_with("blocked:"), "got: {verdict}");
    assert!(verdict.contains(&uid), "the gate must name the finding: {verdict}");
    assert!(
        verdict.contains("disposition_finding"),
        "the gate must tell HANDS how to resolve it: {verdict}"
    );
}

#[tokio::test]
async fn both_dispositions_clear_the_gate() {
    // `fixed` and `rebutted` both clear. A rebuttal deliberately does NOT need
    // the reviewer's agreement — that is what stops the gate deadlocking — so
    // if a future quorum rule made rebuttal require assent, it would be a
    // behaviour change, not a refinement.
    for status in [FindingStatus::Fixed, FindingStatus::Rebutted] {
        let bridge = bridge_with_session("s1").await;
        let uid = bridge
            .eyes_flag(
                "s1".into(),
                "rain".into(),
                FindingSeverity::Blocking,
                format!("finding {status:?}"),
                None,
            )
            .await
            .unwrap();
        assert!(bridge.check_open_findings("s1").await.unwrap().starts_with("blocked:"));

        bridge
            .disposition_finding(uid, status, "because".into(), "brian".into())
            .await
            .unwrap();
        assert_eq!(
            bridge.check_open_findings("s1").await.unwrap(),
            "ok",
            "{status:?} must clear the gate"
        );
    }
}

#[tokio::test]
async fn the_reviewer_down_gate_blocks_only_a_duo_with_a_dead_reviewer() {
    // Fail-closed backstop: a reviewer that is gone cannot have reviewed, so the
    // commit is blocked — but ONLY in a duo, only when the reviewer is really
    // down (health says stalled/dead AND no recent RPC), and HANDS can override.
    //
    // B6 restates this as "every participant holding FileFinding is
    // dead/stalled/absent". Pinned through the OBSERVABLE gate rather than the
    // private decision fn, so the assertion survives the rewrite regardless of
    // how the predicate is factored.
    let bridge = bridge_with_session("s1").await;

    // Healthy reviewer (no health transition reported yet) → no block.
    assert_eq!(bridge.check_open_findings("s1").await.unwrap(), "ok");

    // Reviewer reported dead, no RPC activity → fail closed.
    bridge.notify_agent_health("s1".into(), "rain", "dead");
    let blocked = bridge.check_open_findings("s1").await.unwrap();
    assert!(blocked.starts_with("blocked: reviewer down"), "got: {blocked}");
    assert!(
        blocked.contains("REVIEWER IS GONE, not"),
        "must distinguish reviewer-gone from unreviewed: {blocked}"
    );

    // HANDS overrides → gate opens, and says so rather than silently passing.
    bridge.override_reviewer_block("s1", "confirmed safe to ship unreviewed");
    let overridden = bridge.check_open_findings("s1").await.unwrap();
    assert!(
        overridden.starts_with("ok (reviewer-down overridden"),
        "an override must be visible in the verdict, not silent: {overridden}"
    );

    // Recovery: a reviewer back to running clears the block without an override.
    let bridge2 = bridge_with_session("s2").await;
    bridge2.notify_agent_health("s2".into(), "rain", "dead");
    assert!(bridge2.check_open_findings("s2").await.unwrap().starts_with("blocked:"));
    bridge2.notify_agent_health("s2".into(), "rain", "running");
    assert_eq!(bridge2.check_open_findings("s2").await.unwrap(), "ok");
}

// ---------------------------------------------------------------------------
// THE ACCEPTANCE GATE — the capability gate vs the name gate it replaced
//
// The reframe contract's rule 1 in executable form: rc3 moves the SOURCE of the
// tool gate from an agent name to the caller's capability set, and the DECISION
// must not move with it. This walks the real `tools/list` registry — so a tool
// added later is covered automatically instead of being forgotten — and asserts
// per (tool, agent) that the live gate agrees with `legacy_name_gate_admits`,
// the frozen transcription of the pre-rc3 code.
//
// Every input is real: the roster comes from `ensure_session_roster`, the grants
// come from the migrated `roles` seed via `resolve_caller_capabilities`, and the
// verdict comes from `dispatch`. Nothing here is asserted against a hand-built
// capability set.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_capability_gate_reproduces_the_name_gate_for_every_tool() {
    let bridge = seeded_bridge().await;
    let mut checked = 0usize;
    for descriptor in super::protocol::tool_descriptors() {
        let tool = descriptor.name;
        for agent in ["brian", "rain"] {
            let now = gate_admits(&bridge, tool, agent).await;
            let before = legacy_name_gate_admits(tool, agent);
            assert_eq!(
                now, before,
                "DIVERGENCE on {tool} for {agent}: the capability gate says \
                 admit={now}, the name gate it replaced said admit={before}. \
                 rc3 is a reframe — the source of this decision may move, the \
                 decision may not. Either the seeded capability set is wrong or \
                 the old gate did something the capability model does not \
                 express; both are the user's call."
            );
        }
        checked += 1;
    }
    assert!(
        checked >= 40,
        "expected the full tool registry, only checked {checked} — did \
         tool_descriptors() shrink?"
    );
}

#[tokio::test]
async fn the_seeded_roster_resolves_to_the_presets() {
    // The join that makes every preset-based assertion elsewhere honest. The
    // capability tests in `agents::capability` and the caller helpers in
    // `jsonrpc::tests` all reason about `preset_hands()` / `preset_eyes()`; this
    // is the one place that checks a migrated database actually produces them.
    //
    // `hands` is seeded with a stray `route_gated_command` slug that
    // `Capability::parse` does not know (migration 0044). `from_json` DROPS
    // unrecognised slugs, so it decodes to the preset regardless — this test
    // pins that, which is why the stray slug is noise rather than a divergence.
    use crate::agents::{CapabilitySet, ResolvedCapabilities};
    let bridge = seeded_bridge().await;

    assert_eq!(
        resolve_caller_capabilities(&bridge, "s-parity", "brian").await,
        ResolvedCapabilities::Known(CapabilitySet::preset_hands()),
        "the seeded HANDS role must decode to preset_hands()"
    );
    assert_eq!(
        resolve_caller_capabilities(&bridge, "s-parity", "rain").await,
        ResolvedCapabilities::Known(CapabilitySet::preset_eyes()),
        "the seeded EYES role must decode to preset_eyes()"
    );
}

#[tokio::test]
async fn the_parity_hold_is_exactly_the_known_divergence() {
    // `close_session` is the ONE tool where the capability model and the name
    // gate disagree, and `jsonrpc::PARITY_HOLD` keeps the name gate's answer so
    // the reframe ships without a behaviour change. Two things are pinned:
    //
    //  1. the divergence is real — the model would refuse EYES,
    //  2. the runtime does NOT act on it — EYES can still close.
    //
    // If someone later decides EYES should lose `close_session`, this test is
    // where that conversation starts: delete the hold, move `close_session` off
    // `UNGATED`, and this test's first half becomes the new behaviour.
    use crate::agents::CapabilitySet;

    assert!(
        !CapabilitySet::preset_eyes().allows_tool("close_session"),
        "the capability model must still refuse close_session to EYES — \
         without that there is nothing to hold"
    );
    let bridge = seeded_bridge().await;
    assert!(
        gate_admits(&bridge, "close_session", "rain").await,
        "close_session must stay admitted for EYES until the boundary change is \
         taken deliberately"
    );
    assert!(gate_admits(&bridge, "close_session", "brian").await);

    // And the hold must not quietly grow into a place to park regressions.
    let held: Vec<&str> = super::protocol::tool_descriptors()
        .iter()
        .map(|d| d.name)
        .filter(|t| {
            crate::agents::capability::required_for(t).is_some()
                && CapabilitySet::preset_hands().allows_tool(t)
                    != legacy_name_gate_admits(t, "brian")
                || crate::agents::capability::required_for(t).is_some()
                    && CapabilitySet::preset_eyes().allows_tool(t)
                        != legacy_name_gate_admits(t, "rain")
        })
        .collect();
    assert_eq!(
        held,
        vec!["close_session"],
        "the capability model diverges from the name gate on exactly one tool; \
         a new entry here is an unreviewed behaviour change"
    );
}

#[tokio::test]
async fn an_unreadable_roster_refuses_gated_tools_and_leaves_ungated_ones_alone() {
    // The degradation decision, asserted rather than described. A bridge with no
    // storage is the shape every read failure takes at the gate: no participant
    // row, a corrupt `capabilities` column, storage not yet wired.
    //
    // Fail CLOSED on gated tools — a gate that cannot read its policy must not
    // guess, and guessing "allow" would silently hand EYES the executor's tools.
    // Fail OPEN on ungated ones — they were never gated, so a roster failure
    // must not start gating them.
    let bridge = SignalingBridge::new(); // storage never set
    for agent in ["brian", "rain"] {
        for tool in HANDS_ONLY.iter().chain(EYES_ONLY).chain(CL_MUTATE) {
            assert!(
                rejected_by(&bridge, tool, agent).await,
                "{tool} must be refused for {agent} when the roster cannot be read"
            );
        }
        for tool in UNGATED {
            assert!(
                !rejected_by(&bridge, tool, agent).await,
                "{tool} is ungated — an unreadable roster must not start gating it"
            );
        }
    }

    // The refusal has to be diagnosable from the transcript alone, or an agent
    // reports "a tool failed" and the cause is invisible.
    let who = caller(&bridge, "brian").await;
    let out = dispatch(call("ask_user_choice"), &who, &bridge).await.unwrap();
    let rendered = serde_json::to_value(&out).unwrap().to_string();
    assert!(
        rendered.contains("could not read your capability set"),
        "the refusal must say the set could not be read: {rendered}"
    );
    assert!(
        rendered.contains("ask_user"),
        "the refusal must name the capability it needed: {rendered}"
    );
    // And the specific reason, interpolated — "could not be read" alone tells
    // nobody whether the roster is missing, corrupt, or simply not wired yet,
    // which is the whole difference between "restart" and "repair the row".
    assert!(
        rendered.contains("storage is not wired up yet"),
        "the refusal must carry the resolver's reason, not just that there was one: {rendered}"
    );
}

#[tokio::test]
async fn the_three_gate_lists_are_disjoint() {
    // A tool in two lists would make its authorization order-dependent — the
    // kind of latent ambiguity a capability rewrite would inherit silently.
    for t in HANDS_ONLY {
        assert!(!EYES_ONLY.contains(t), "{t} in both HANDS and EYES lists");
        assert!(!UNGATED.contains(t), "{t} in both HANDS and UNGATED lists");
    }
    for t in EYES_ONLY {
        assert!(!CL_MUTATE.contains(t), "{t} in both EYES and CL_MUTATE lists");
        assert!(!UNGATED.contains(t), "{t} in both EYES and UNGATED lists");
    }
}
