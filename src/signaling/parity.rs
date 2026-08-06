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
