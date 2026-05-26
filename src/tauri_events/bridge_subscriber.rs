//! Routes `SignalingEvent`s from the bridge into Tauri emit calls.
//!
//! The bridge fires `SignalingEvent` over a `tokio::sync::broadcast::Sender`.
//! This subscriber picks each one up and dispatches:
//!
//! - `MessagePersisted { session_id, message_id }` → `BatchEmitter::touch(...)`
//!   (coalesced fetch, single emit per batch).
//! - `PendingChoice`, `ChoiceResolved`, `AwaitingUser`, `AgentAdvancePhase`
//!   → direct `emit_fn(name, payload)` (no batching — these are infrequent).
//! - `SessionCloseRequest` → ignored here. AppState's own subscriber handles
//!   shutdown; the frontend gets notified via the downstream session row
//!   update once the close path completes.
//!
//! The emit function is a closure so this module is testable without a
//! running Tauri runtime. Batch 4's main.rs wires it to
//! `app.emit(name, &serde_json::to_value(payload).unwrap())`.

use crate::signaling::{SignalingBridge, SignalingEvent};
use crate::storage::Storage;
use crate::tauri_events::batch_emitter::BatchEmitter;
use crate::tauri_events::types::{
    AwaitingUser, ChoiceResolvedEvent, PhaseChangedEvent,
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
            let v = serde_json::json!({
                "choice_id": p.choice_id,
                "session_id": p.session_id,
                "agent": p.agent,
                "question": p.question,
                "options": p.options,
            });
            emit_event("session:pending_choice", v);
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
            // Handled by AppState's existing subscriber path. UI sees the
            // close downstream via the session row update.
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

        bridge.notify_message_persisted("s1".to_string(), id);
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
}
