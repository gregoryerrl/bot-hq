//! Routes `SignalingEvent`s from the bridge into Tauri emit calls.
//!
//! The bridge fires `SignalingEvent` over a `tokio::sync::broadcast::Sender`.
//! This subscriber picks each one up and dispatches:
//!
//! - `MessagePersisted { session_id, message_id }` → `BatchEmitter::touch(...)`
//!   (coalesced fetch, single emit per batch).
//! - `PendingChoice`, `ChoiceResolved`, `AwaitingUser`, `AgentAdvancePhase`
//!   → direct `emit_fn(name, payload)` (no batching — these are infrequent).
//! - `SessionCloseRequest` → ignored here. A dedicated handler in main.rs
//!   (with an `Arc<CoreAppState>`) routes it to `core.close_session`; the
//!   frontend gets notified via the downstream session row update once the
//!   close path completes.
//!
//! The emit function is a closure so this module is testable without a
//! running Tauri runtime. Batch 4's main.rs wires it to
//! `app.emit(name, &serde_json::to_value(payload).unwrap())`.

use crate::signaling::{SignalingBridge, SignalingEvent};
use crate::storage::Storage;
use crate::tauri_events::batch_emitter::BatchEmitter;
use crate::tauri_events::types::{
    AgentHealthEvent, AwaitingUser, ChoiceResolvedEvent, DocChangedEvent,
    FindingsChangedEvent, PendingChoiceEvent, PhaseChangedEvent, RouterHealthEvent,
    SessionActivityEvent, SessionClosedEvent,
};
use serde_json::Value;
use std::sync::Arc;

/// Trait alias for the emit closure — `Fn(event_name, serialized_payload)`.
pub trait EmitFn: Fn(&str, Value) + Send + Sync + 'static {}
impl<T> EmitFn for T where T: Fn(&str, Value) + Send + Sync + 'static {}

/// Spawn the bridge → Tauri event router. Returns immediately; the routing
/// happens in a tokio task that owns the broadcast `Receiver`.
pub fn spawn_subscriber<EM, EB>(
    bridge: Arc<SignalingBridge>,
    storage: Arc<Storage>,
    emit_msg_batch: EM,
    emit_event: EB,
) where
    EM: Fn(Vec<crate::tauri_events::types::AgentMessage>) + Send + Sync + 'static,
    EB: EmitFn,
{
    let mut rx = bridge.subscribe();
    let emitter = BatchEmitter::new(emit_msg_batch, storage);
    let emit_event = Arc::new(emit_event);

    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => route(ev, &emitter, emit_event.as_ref()),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(
                        skipped,
                        "tauri bridge subscriber lagged; broadcast receiver skipped events"
                    );
                    // We may have dropped session:* events the UI relies on. Emit
                    // a resync so the frontend refetches its event-backed queries
                    // and a lagged burst can't leave the UI stale — this is what
                    // lets us drop the redundant safety-net refetch polls.
                    emit_event.as_ref()("session:resync", Value::Null);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn route<EB: EmitFn + ?Sized>(ev: SignalingEvent, emitter: &BatchEmitter, emit_event: &EB) {
    match ev {
        SignalingEvent::MessagePersisted { session_id, message_id: _ } => {
            emitter.touch(session_id);
        }
        SignalingEvent::PendingChoice(p) => {
            let payload = PendingChoiceEvent {
                choice_id: p.choice_id,
                session_id: p.session_id,
                agent: p.agent,
                question: p.question,
                options: p.options,
            };
            emit_event(
                PendingChoiceEvent::EVENT_NAME,
                serde_json::to_value(&payload).unwrap_or(Value::Null),
            );
        }
        SignalingEvent::ChoiceResolved { choice_id, picked } => {
            let payload = ChoiceResolvedEvent { choice_id, picked };
            emit_event(
                ChoiceResolvedEvent::EVENT_NAME,
                serde_json::to_value(&payload).unwrap_or(Value::Null),
            );
        }
        SignalingEvent::AwaitingUser { session_id, agent, reason } => {
            let payload = AwaitingUser { session_id, agent, reason };
            emit_event(
                AwaitingUser::EVENT_NAME,
                serde_json::to_value(&payload).unwrap_or(Value::Null),
            );
        }
        SignalingEvent::AgentAdvancePhase { session_id, agent, target } => {
            let payload = PhaseChangedEvent { session_id, agent, target };
            emit_event(
                PhaseChangedEvent::EVENT_NAME,
                serde_json::to_value(&payload).unwrap_or(Value::Null),
            );
        }
        SignalingEvent::SessionCloseRequest { .. } => {
            // Not this subscriber's job — the dedicated close handler in
            // main.rs (which owns Arc<CoreAppState>) routes this to
            // core.close_session. That, once done, fires `SessionClosed`
            // (below) which IS what the UI reacts to.
        }
        SignalingEvent::DocChanged { session_id } => {
            let payload = DocChangedEvent { session_id };
            emit_event(
                DocChangedEvent::EVENT_NAME,
                serde_json::to_value(&payload).unwrap_or(Value::Null),
            );
        }
        SignalingEvent::FindingsChanged { session_id } => {
            let payload = FindingsChangedEvent { session_id };
            emit_event(
                FindingsChangedEvent::EVENT_NAME,
                serde_json::to_value(&payload).unwrap_or(Value::Null),
            );
        }
        SignalingEvent::SessionClosed { session_id } => {
            let payload = SessionClosedEvent { session_id };
            emit_event(
                SessionClosedEvent::EVENT_NAME,
                serde_json::to_value(&payload).unwrap_or(Value::Null),
            );
        }
        SignalingEvent::HaltsCleared { session_id: _ } => {
            // Pending awaiting-halts were answered; invalidate the tray so the
            // "needs input" bell clears. Null payload — the frontend just
            // refetches list_pending_tray (it isn't per-session data).
            emit_event("session:halt_cleared", Value::Null);
        }
        SignalingEvent::AgentHealth {
            session_id,
            agent,
            health,
        } => {
            let payload = AgentHealthEvent {
                session_id,
                agent,
                health,
            };
            emit_event(
                AgentHealthEvent::EVENT_NAME,
                serde_json::to_value(&payload).unwrap_or(Value::Null),
            );
        }
        SignalingEvent::SessionActivity {
            session_id,
            state,
            brian_busy,
            rain_busy,
        } => {
            let payload = SessionActivityEvent {
                session_id,
                state,
                brian_busy,
                rain_busy,
            };
            emit_event(
                SessionActivityEvent::EVENT_NAME,
                serde_json::to_value(&payload).unwrap_or(Value::Null),
            );
        }
        SignalingEvent::RouterHealth { session_id, alive } => {
            let payload = RouterHealthEvent { session_id, alive };
            emit_event(
                RouterHealthEvent::EVENT_NAME,
                serde_json::to_value(&payload).unwrap_or(Value::Null),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signaling::SignalingBridge;
    use crate::storage::{Author, MessageKind, Storage};
    use std::sync::Mutex;
    use std::time::Duration;

    async fn test_storage() -> Arc<Storage> {
        let s = Storage::memory().await.unwrap();
        s.create_session("s1", "test", None).await.unwrap();
        Arc::new(s)
    }

    #[tokio::test]
    async fn routes_message_persisted_to_emitter() {
        let bridge = SignalingBridge::new();
        let storage = test_storage().await;
        let id = storage
            .insert_message("s1", Author::Brian, MessageKind::Text, "hi")
            .await
            .unwrap();
        let captured_msgs: Arc<Mutex<Vec<Vec<crate::tauri_events::types::AgentMessage>>>> =
            Arc::new(Mutex::new(Vec::new()));
        let cap = captured_msgs.clone();

        spawn_subscriber(
            bridge.clone(),
            storage.clone(),
            move |msgs| cap.lock().unwrap().push(msgs),
            |_name: &str, _payload: Value| {},
        );

        // Yield to let the spawn_subscriber task subscribe to the broadcast.
        tokio::time::sleep(Duration::from_millis(10)).await;

        bridge.notify_message_persisted("s1".into(), id);
        tokio::time::sleep(Duration::from_millis(150)).await;

        let captured = captured_msgs.lock().unwrap();
        assert!(!captured.is_empty(), "expected at least one batch");
        assert_eq!(captured[0][0].content, "hi");
    }

    #[tokio::test]
    async fn route_awaiting_user_emits_typed_event() {
        let storage = test_storage().await;
        let captured_events: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let ev_cap = captured_events.clone();

        let emitter = BatchEmitter::new(|_| {}, storage);
        let ev = SignalingEvent::AwaitingUser {
            session_id: "s1".into(),
            agent: "brian".into(),
            reason: "test reason".into(),
        };

        route(ev, &emitter, &move |name: &str, payload: Value| {
            ev_cap.lock().unwrap().push((name.to_string(), payload));
        });

        let captured = captured_events.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].0, AwaitingUser::EVENT_NAME);
        assert_eq!(captured[0].1["session_id"], "s1");
        assert_eq!(captured[0].1["reason"], "test reason");
    }

    #[tokio::test]
    async fn route_agent_advance_phase_emits_typed_event() {
        let storage = test_storage().await;
        let captured_events: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let ev_cap = captured_events.clone();

        let emitter = BatchEmitter::new(|_| {}, storage);
        let ev = SignalingEvent::AgentAdvancePhase {
            session_id: "s1".into(),
            agent: "brian".into(),
            target: "Apply".into(),
        };

        route(ev, &emitter, &move |name: &str, payload: Value| {
            ev_cap.lock().unwrap().push((name.to_string(), payload));
        });

        let captured = captured_events.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].0, PhaseChangedEvent::EVENT_NAME);
        assert_eq!(captured[0].1["target"], "Apply");
    }

    #[tokio::test]
    async fn route_doc_changed_emits_typed_event() {
        let storage = test_storage().await;
        let captured_events: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let ev_cap = captured_events.clone();
        let emitter = BatchEmitter::new(|_| {}, storage);
        let ev = SignalingEvent::DocChanged { session_id: "s1".into() };
        route(ev, &emitter, &move |name: &str, payload: Value| {
            ev_cap.lock().unwrap().push((name.to_string(), payload));
        });
        let captured = captured_events.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].0, DocChangedEvent::EVENT_NAME);
        assert_eq!(captured[0].1["session_id"], "s1");
    }

    #[tokio::test]
    async fn route_session_closed_emits_typed_event() {
        let storage = test_storage().await;
        let captured_events: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let ev_cap = captured_events.clone();
        let emitter = BatchEmitter::new(|_| {}, storage);
        let ev = SignalingEvent::SessionClosed { session_id: "s1".into() };
        route(ev, &emitter, &move |name: &str, payload: Value| {
            ev_cap.lock().unwrap().push((name.to_string(), payload));
        });
        let captured = captured_events.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].0, SessionClosedEvent::EVENT_NAME);
        assert_eq!(captured[0].1["session_id"], "s1");
    }

    #[tokio::test]
    async fn route_agent_health_emits_typed_event() {
        // B2: a supervisor liveness change must reach the frontend as
        // `session:agent_health` with the agent + health string intact.
        let storage = test_storage().await;
        let captured_events: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let ev_cap = captured_events.clone();
        let emitter = BatchEmitter::new(|_| {}, storage);
        let ev = SignalingEvent::AgentHealth {
            session_id: "s1".into(),
            agent: "brian".into(),
            health: "retrying".into(),
        };
        route(ev, &emitter, &move |name: &str, payload: Value| {
            ev_cap.lock().unwrap().push((name.to_string(), payload));
        });
        let captured = captured_events.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].0, AgentHealthEvent::EVENT_NAME);
        assert_eq!(captured[0].1["agent"], "brian");
        assert_eq!(captured[0].1["health"], "retrying");
    }

    #[tokio::test]
    async fn route_halts_cleared_emits_tray_event() {
        // A cleared awaiting-halt must reach the frontend as `session:halt_cleared`
        // so GlobalEventSync invalidates TRAY_KEYS and the bell badge clears.
        let storage = test_storage().await;
        let captured_events: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let ev_cap = captured_events.clone();
        let emitter = BatchEmitter::new(|_| {}, storage);
        let ev = SignalingEvent::HaltsCleared { session_id: "s1".into() };
        route(ev, &emitter, &move |name: &str, payload: Value| {
            ev_cap.lock().unwrap().push((name.to_string(), payload));
        });
        let captured = captured_events.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].0, "session:halt_cleared");
    }
}
