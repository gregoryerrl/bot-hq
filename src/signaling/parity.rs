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

use super::jsonrpc::{dispatch, CallerIdentity};
use super::protocol::JsonRpcRequest;
use crate::signaling::SignalingBridge;
use serde_json::json;

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
    "advance_phase",
    "gate_status",
];

fn caller(agent: &str) -> CallerIdentity {
    CallerIdentity {
        session_id: "s-parity".into(),
        agent: agent.into(),
    }
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

/// Did this dispatch come back as a ROLE rejection (as opposed to succeeding, or
/// failing for an unrelated reason like absent storage)? Keyed on the wording
/// each gate emits in `jsonrpc.rs:208-224`.
async fn role_rejected(tool: &str, agent: &str) -> bool {
    let bridge = SignalingBridge::new();
    let out = dispatch(call(tool), &caller(agent), &bridge).await;
    let rendered = match out {
        Ok(Some(res)) => serde_json::to_value(&res).map(|v| v.to_string()).unwrap_or_default(),
        Ok(None) => String::new(),
        Err(e) => format!("{e:?}"),
    };
    rendered.contains("is reserved for the HANDS agent")
        || rendered.contains("is reserved for HANDS (brian)")
        || rendered.contains("is reserved for the EYES agent")
}

#[tokio::test]
async fn hands_only_tools_reject_eyes_and_admit_hands() {
    for tool in HANDS_ONLY {
        assert!(
            role_rejected(tool, "rain").await,
            "{tool} must reject EYES — it is HANDS-only today"
        );
        assert!(
            !role_rejected(tool, "brian").await,
            "{tool} must NOT be role-rejected for HANDS"
        );
    }
}

#[tokio::test]
async fn eyes_only_tools_reject_hands_and_admit_eyes() {
    for tool in EYES_ONLY {
        assert!(
            role_rejected(tool, "brian").await,
            "{tool} must reject HANDS — EYES files findings, HANDS resolves them"
        );
        assert!(
            !role_rejected(tool, "rain").await,
            "{tool} must NOT be role-rejected for EYES"
        );
    }
}

#[tokio::test]
async fn cl_mutating_tools_reject_eyes_and_admit_hands() {
    for tool in CL_MUTATE {
        assert!(
            role_rejected(tool, "rain").await,
            "{tool} must reject EYES — HANDS owns CL content authorship"
        );
        assert!(
            !role_rejected(tool, "brian").await,
            "{tool} must NOT be role-rejected for HANDS"
        );
    }
}

#[tokio::test]
async fn ungated_tools_admit_both_roles() {
    // The other direction: the capability rewrite must not silently RESTRICT a
    // tool both roles can use today. Over-gating is as much a parity break as
    // under-gating, and is easier to ship unnoticed.
    for tool in UNGATED {
        for agent in ["brian", "rain"] {
            assert!(
                !role_rejected(tool, agent).await,
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
// CROSS-CHECK — the capability model vs the live dispatch layer
//
// B0.1 pins what `dispatch` does. B2's tests pin what `CapabilitySet` decides.
// Neither proves the two AGREE, and that agreement is the entire premise of B6:
// swapping name-equality for capability lookup is only safe if, for every tool
// in the registry, both reach the same verdict for both roles.
//
// This runs the real `tools/list` registry — so a tool added later is covered
// automatically instead of being forgotten.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_capability_model_reproduces_the_dispatch_layer_exactly() {
    use crate::agents::capability::CapabilitySet;

    let presets = [
        ("brian", CapabilitySet::preset_hands()),
        ("rain", CapabilitySet::preset_eyes()),
    ];
    // The single deliberate difference, asserted in
    // `agents::capability::tests::close_session_is_the_one_intended_boundary_change`:
    // `close_session` is ungated today (CL issues #5) and becomes a capability.
    const INTENDED_DIVERGENCE: &[&str] = &["close_session"];

    let mut checked = 0usize;
    for descriptor in super::protocol::tool_descriptors() {
        let tool = descriptor.name;
        if INTENDED_DIVERGENCE.contains(&tool) {
            continue;
        }
        for (agent, caps) in &presets {
            let dispatch_admits = !role_rejected(tool, agent).await;
            let model_admits = caps.allows_tool(tool);
            assert_eq!(
                model_admits, dispatch_admits,
                "DIVERGENCE on {tool} for {agent}: capability model says \
                 {model_admits}, dispatch says {dispatch_admits}. B6 would \
                 change behaviour here."
            );
        }
        checked += 1;
    }
    assert!(
        checked >= 30,
        "expected the full tool registry, only checked {checked} — did \
         tool_descriptors() shrink?"
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
