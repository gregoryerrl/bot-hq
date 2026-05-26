//! Coalescing emitter for agent-message events.
//!
//! Subscribes to `MessagePersisted` IDs (via [`crate::tauri_events::
//! bridge_subscriber::spawn_subscriber`]), tracks a per-session `since_id`
//! watermark, and flushes on the earlier of: 20 touches, 50ms timer, or
//! explicit `flush()`. Each flush calls `storage.messages_for_session(sid,
//! since_id)` per dirty session — one indexed SELECT per session.
//!
//! Operates over an `mpsc::UnboundedChannel` so callers don't await the
//! flush; the spawned task owns the state and runs until the sender drops.

use crate::storage::Storage;
use crate::tauri_events::types::AgentMessage;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;

/// Default coalesce threshold. Reaching this many touches between flushes
/// triggers an immediate fetch + emit (bypasses the 50ms timer).
const FLUSH_AT_N: usize = 20;
/// Default flush window. Once a touch arrives and no timer is active, the
/// emitter schedules a flush this far in the future.
const FLUSH_WINDOW: Duration = Duration::from_millis(50);

#[derive(Debug)]
enum EmitMsg {
    Touch { session_id: String },
    Flush,
}

/// Hot-path emitter for agent messages. Cheap to clone — internals share an
/// `Arc<UnboundedSender>`.
#[derive(Clone)]
pub struct BatchEmitter {
    tx: mpsc::UnboundedSender<EmitMsg>,
}

impl BatchEmitter {
    /// Spawn a background task that owns the watermark map + dirty set.
    /// `emit_fn` receives each batched `Vec<AgentMessage>`; in Batch 4 the
    /// caller wires this to `app.emit(AgentMessage::EVENT_NAME_BATCH, &v)`.
    pub fn new<F>(emit_fn: F, storage: Arc<Storage>) -> Self
    where
        F: Fn(Vec<AgentMessage>) + Send + Sync + 'static,
    {
        let (tx, rx) = mpsc::unbounded_channel();
        let emit_fn = Arc::new(emit_fn);
        tokio::spawn(run_loop(rx, emit_fn, storage));
        Self { tx }
    }

    /// Signal that `session_id` has new messages. Fire-and-forget; returns
    /// immediately. Drops silently if the receiver task has exited
    /// (BatchEmitter dropped or shutdown signal).
    pub fn touch(&self, session_id: String) {
        let _ = self.tx.send(EmitMsg::Touch { session_id });
    }

    /// Force an immediate flush. Used for tests + (deferred Path B) turn-end
    /// signal from per-agent pumps.
    pub fn flush(&self) {
        let _ = self.tx.send(EmitMsg::Flush);
    }
}

async fn run_loop<F>(
    mut rx: mpsc::UnboundedReceiver<EmitMsg>,
    emit_fn: Arc<F>,
    storage: Arc<Storage>,
) where
    F: Fn(Vec<AgentMessage>) + Send + Sync + 'static,
{
    let mut watermarks: HashMap<String, i64> = HashMap::new();
    let mut dirty: HashSet<String> = HashSet::new();
    let mut touches_since_flush: usize = 0;
    let mut flush_at: Option<Instant> = None;

    loop {
        let msg = match flush_at {
            Some(t) => tokio::select! {
                m = rx.recv() => m,
                _ = tokio::time::sleep_until(t) => {
                    flush_once(&storage, &mut watermarks, &mut dirty, &emit_fn).await;
                    touches_since_flush = 0;
                    flush_at = None;
                    continue;
                }
            },
            None => match rx.recv().await {
                Some(m) => Some(m),
                None => return, // sender dropped; shut down
            },
        };

        let Some(msg) = msg else { return };

        match msg {
            EmitMsg::Touch { session_id } => {
                dirty.insert(session_id);
                touches_since_flush += 1;
                if touches_since_flush >= FLUSH_AT_N {
                    flush_once(&storage, &mut watermarks, &mut dirty, &emit_fn).await;
                    touches_since_flush = 0;
                    flush_at = None;
                } else if flush_at.is_none() {
                    flush_at = Some(Instant::now() + FLUSH_WINDOW);
                }
            }
            EmitMsg::Flush => {
                flush_once(&storage, &mut watermarks, &mut dirty, &emit_fn).await;
                touches_since_flush = 0;
                flush_at = None;
            }
        }
    }
}

async fn flush_once<F>(
    storage: &Storage,
    watermarks: &mut HashMap<String, i64>,
    dirty: &mut HashSet<String>,
    emit_fn: &Arc<F>,
) where
    F: Fn(Vec<AgentMessage>) + Send + Sync + 'static,
{
    if dirty.is_empty() {
        return;
    }
    let mut all_msgs: Vec<AgentMessage> = Vec::new();
    let session_ids: Vec<String> = dirty.drain().collect();
    for sid in session_ids {
        let since = watermarks.get(&sid).copied();
        match storage.messages_for_session(&sid, since).await {
            Ok(msgs) => {
                if let Some(last) = msgs.last() {
                    watermarks.insert(sid, last.id);
                }
                all_msgs.extend(msgs.into_iter().map(AgentMessage::from));
            }
            Err(e) => {
                tracing::warn!(error = ?e, "BatchEmitter: messages_for_session failed");
            }
        }
    }
    if !all_msgs.is_empty() {
        emit_fn(all_msgs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{Author, MessageKind, Storage};
    use std::sync::Mutex;

    async fn test_storage_with_messages(
        session_id: &str,
        contents: &[&str],
    ) -> Arc<Storage> {
        let s = Storage::memory().await.unwrap();
        s.create_session(session_id, "test", None).await.unwrap();
        for c in contents {
            s.insert_message(session_id, Author::Brian, MessageKind::Text, c)
                .await
                .unwrap();
        }
        Arc::new(s)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn batch_emitter_fetches_after_timer_window() {
        let storage = test_storage_with_messages("s1", &["hello", "world"]).await;
        let captured = Arc::new(Mutex::new(Vec::new()));
        let cap = captured.clone();
        let emitter = BatchEmitter::new(
            move |msgs| cap.lock().unwrap().push(msgs),
            storage,
        );

        emitter.touch("s1".to_string());
        emitter.touch("s1".to_string());

        // No flush yet — under N=20 + within 50ms window
        tokio::time::sleep(Duration::from_millis(100)).await;

        let captured = captured.lock().unwrap();
        assert_eq!(captured.len(), 1, "expected exactly one batch");
        assert_eq!(captured[0].len(), 2);
        assert_eq!(captured[0][0].content, "hello");
        assert_eq!(captured[0][1].content, "world");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn batch_emitter_watermark_advances_across_flushes() {
        let storage = Arc::new(Storage::memory().await.unwrap());
        storage.create_session("s1", "test", None).await.unwrap();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let cap = captured.clone();
        let emitter = BatchEmitter::new(
            move |msgs| cap.lock().unwrap().push(msgs),
            storage.clone(),
        );

        // First batch: 2 messages
        let id1 = storage
            .insert_message("s1", Author::Brian, MessageKind::Text, "a")
            .await
            .unwrap();
        let id2 = storage
            .insert_message("s1", Author::Brian, MessageKind::Text, "b")
            .await
            .unwrap();
        emitter.touch("s1".to_string());
        let _ = id1;
        let _ = id2;
        emitter.touch("s1".to_string());
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Second batch: 1 new message, should fetch only it (watermark advanced)
        let id3 = storage
            .insert_message("s1", Author::Brian, MessageKind::Text, "c")
            .await
            .unwrap();
        emitter.touch("s1".to_string());
        let _ = id3;
        tokio::time::sleep(Duration::from_millis(100)).await;

        let captured = captured.lock().unwrap();
        assert_eq!(captured.len(), 2, "expected two batches across two flushes");
        assert_eq!(captured[0].len(), 2);
        assert_eq!(captured[1].len(), 1);
        assert_eq!(captured[1][0].content, "c");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn batch_emitter_flushes_immediately_on_explicit_flush() {
        let storage = test_storage_with_messages("s1", &["x"]).await;
        let captured = Arc::new(Mutex::new(Vec::new()));
        let cap = captured.clone();
        let emitter = BatchEmitter::new(
            move |msgs| cap.lock().unwrap().push(msgs),
            storage,
        );

        emitter.touch("s1".to_string());
        emitter.flush();
        tokio::time::sleep(Duration::from_millis(50)).await;

        let captured = captured.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn batch_emitter_no_emit_when_nothing_dirty() {
        let storage = test_storage_with_messages("s1", &["x"]).await;
        let captured = Arc::new(Mutex::new(Vec::new()));
        let cap = captured.clone();
        let emitter = BatchEmitter::new(
            move |msgs| cap.lock().unwrap().push(msgs),
            storage,
        );

        // No touch — flush should be a no-op
        emitter.flush();
        tokio::time::sleep(Duration::from_millis(50)).await;

        assert!(captured.lock().unwrap().is_empty());
    }
}
