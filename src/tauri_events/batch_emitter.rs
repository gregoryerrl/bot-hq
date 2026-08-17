//! Coalescing emitter for agent-message events.
//!
//! Subscribes to `MessagePersisted` IDs (via [`crate::tauri_events::
//! bridge_subscriber::spawn_subscriber`]), tracks a per-session `since_id`
//! watermark, and flushes on the earlier of: 20 touches, 50ms timer, or
//! explicit `flush()`. Each flush calls `storage.messages_for_session(sid,
//! since_id)` per dirty session — one indexed SELECT per session.
//!
//! **A session's first touch after launch seeds its watermark from the id it
//! carries** (round 7). The map starts empty at every launch, and an unseeded
//! `since_id` is `None` — `messages_for_session(sid, None)` is the WHOLE
//! channel, no LIMIT — so the first chunk of any session that survived a
//! restart re-emitted its entire history through Tauri IPC (this session
//! stood at 1,954 rows when it was measured; the 06:48Z rebuild that day did
//! it to three live sessions). The frontend already loads history through
//! `list_messages` on mount, so the replay was duplicate work at best and a
//! multi-MB emit at worst. Seeding to `message_id - 1` emits exactly the rows
//! persisted since launch.
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
    Touch { session_id: Arc<str>, message_id: i64 },
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

    /// Signal that `session_id` has a new message, `message_id`. Fire-and-forget;
    /// returns immediately. Drops silently if the receiver task has exited
    /// (BatchEmitter dropped or shutdown signal). The id seeds the session's
    /// watermark on its FIRST touch after launch — see the module doc.
    pub fn touch(&self, session_id: Arc<str>, message_id: i64) {
        let _ = self.tx.send(EmitMsg::Touch {
            session_id,
            message_id,
        });
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
    let mut watermarks: HashMap<Arc<str>, i64> = HashMap::new();
    let mut dirty: HashSet<Arc<str>> = HashSet::new();
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
            EmitMsg::Touch {
                session_id,
                message_id,
            } => {
                // First touch of this session since launch: everything before
                // this row is history the frontend already has. Seeded to the
                // row BEFORE this one, so this row and everything after it —
                // including rows touched later in the same flush window — go
                // out; nothing older does.
                watermarks
                    .entry(Arc::clone(&session_id))
                    .or_insert(message_id - 1);
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
    watermarks: &mut HashMap<Arc<str>, i64>,
    dirty: &mut HashSet<Arc<str>>,
    emit_fn: &Arc<F>,
) where
    F: Fn(Vec<AgentMessage>) + Send + Sync + 'static,
{
    if dirty.is_empty() {
        return;
    }
    let mut all_msgs: Vec<AgentMessage> = Vec::new();
    // Detach the dirty set so it isn't borrowed across the await below, then swap
    // the emptied set — capacity intact — back at the end, so the next flush reuses
    // it instead of allocating a throwaway `Vec` each time (O4). Draining moves each
    // owned id, reused as the watermark key (no clone).
    let mut pending = std::mem::take(dirty);
    for sid in pending.drain() {
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
    *dirty = pending;
    if !all_msgs.is_empty() {
        emit_fn(all_msgs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{MessageKind, Storage};
    use std::sync::Mutex;

    async fn test_storage_with_messages(
        session_id: &str,
        contents: &[&str],
    ) -> Arc<Storage> {
        let s = Storage::memory().await.unwrap();
        s.create_session(session_id, "test", None).await.unwrap();
        for c in contents {
            s.post_to_channel(session_id, "participant", Some("hands"), MessageKind::Text.as_str(), *c, None)
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

        emitter.touch("s1".into(), 1);
        emitter.touch("s1".into(), 2);

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
            .post_to_channel("s1", "participant", Some("hands"), MessageKind::Text.as_str(), "a", None)
            .await
            .unwrap();
        let id2 = storage
            .post_to_channel("s1", "participant", Some("hands"), MessageKind::Text.as_str(), "b", None)
            .await
            .unwrap();
        emitter.touch("s1".into(), id1.message_id());
        emitter.touch("s1".into(), id2.message_id());
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Second batch: 1 new message, should fetch only it (watermark advanced)
        let id3 = storage
            .post_to_channel("s1", "participant", Some("hands"), MessageKind::Text.as_str(), "c", None)
            .await
            .unwrap();
        emitter.touch("s1".into(), id3.message_id());
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

        emitter.touch("s1".into(), 1);
        emitter.flush();
        tokio::time::sleep(Duration::from_millis(50)).await;

        let captured = captured.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].len(), 1);
    }

    /// The first touch after launch does NOT replay the channel. Three rows are
    /// history (persisted "before this launch"); the fourth is the one just
    /// persisted, and it is the only one that goes out.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn first_touch_after_launch_does_not_replay_history() {
        let storage = test_storage_with_messages("s1", &["old-1", "old-2", "old-3"]).await;
        let fresh = storage
            .post_to_channel("s1", "participant", Some("hands"), MessageKind::Text.as_str(), "new", None)
            .await
            .unwrap();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let cap = captured.clone();
        let emitter = BatchEmitter::new(
            move |msgs| cap.lock().unwrap().push(msgs),
            storage,
        );
        emitter.touch("s1".into(), fresh.message_id());
        emitter.flush();
        tokio::time::sleep(Duration::from_millis(50)).await;

        let captured = captured.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(
            captured[0].iter().map(|m| m.content.as_str()).collect::<Vec<_>>(),
            vec!["new"],
            "only the row that was persisted since launch goes out"
        );
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
