//! The control-event journal (Plan 5c-1): ONE task subscribed to the bridge's
//! own `SignalingEvent` broadcast — the stream the desktop UI already consumes
//! — appends a `control_events` row per event a controller may care about, so
//! `app_control events` can page them forward by id (`storage::control_events`).
//! A single join point, no emit scattered through the writers.
//!
//! What it keeps: a message row (tool rows excluded; a delegated message
//! carries the controller that sent it as its source), a decision parked or
//! resolved, a halt declared or cleared, a session closed, a participant's
//! health, an activity transition (a repeated state once), an urgent gate, a
//! phase vote, a document or findings change, a roster change. What it does
//! not: configuration, context meters, attention toggles, staging plumbing.
//!
//! Durability is the broadcast's, which starts AFTER the source row exists and
//! is fire-and-forget (eyes-2, Plan 5c review): a process that dies between
//! the source write and the journal insert loses the event, and a new receiver
//! after a restart knows nothing of it. So the task writes a `journal_restart`
//! marker when it starts and RECONCILES the two durable attention states —
//! EVERY pending decision and standing halt of every open session, deduped by
//! a stable identity (the choice id; the halt's declared-at stamp, carried as
//! `ref_id`) against what the journal already holds — and a lagged receiver
//! writes a `journal_gap` row with the count it dropped. No time cutoff: the
//! newest journal row proves nothing about what was covered (a crash halfway
//! through a catch-up, or an older source event beside a newer unrelated row,
//! would otherwise be skipped for good). The same identity check keeps a
//! `PendingChoice` buffered during the catch-up from landing twice. A reader
//! sees the discontinuity instead of a smooth story; everything else (a
//! message, an activity flip) may be missing across a crash, and the contract
//! says so.
//!
//! Cost without a controller: the chatty kinds (`message`, `activity`) are
//! journaled only while some enabled participant of an open session holds the
//! application-control grant (re-read every 30 s and on roster changes); the
//! attention kinds are rare and always kept, which is what reconciliation
//! rebuilds anyway.

use super::*;
use crate::storage::{AttentionRow, NewControlEvent, CONTROL_EVENT_MAX_ROWS, CONTROL_EVENT_RETENTION_DAYS, JOURNAL_SESSION};
use std::sync::atomic::AtomicU32;
/// Detail text is cut here: a reader wanting the whole thing reads the row.
const DETAIL_BYTES: usize = 200;
/// How often the retention sweep runs, in inserts.
const SWEEP_EVERY: u32 = 200;
/// How long the controller-presence answer is trusted.
const PRESENCE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// The journal's live state — the task's and, in tests, the caller's.
pub struct ControlEventJournal {
    bridge: Weak<SignalingBridge>,
    last_activity: Mutex<HashMap<String, String>>,
    presence: Mutex<Option<(bool, std::time::Instant)>>,
    inserts: AtomicU32,
    /// The watches and their wake (Plan 5c-2): every row written goes to it.
    watches: super::control_watches::WatchEngine,
}

fn cut(text: &str) -> String {
    let redacted = crate::policy::secret_scan::redact(text);
    let end = crate::text::floor_char_boundary(&redacted, DETAIL_BYTES.min(redacted.len()));
    redacted[..end].to_string()
}

impl ControlEventJournal {
    fn new(bridge: &Arc<SignalingBridge>) -> Arc<Self> {
        Arc::new(Self {
            bridge: Arc::downgrade(bridge),
            last_activity: Mutex::new(HashMap::new()),
            presence: Mutex::new(None),
            inserts: AtomicU32::new(0),
            watches: super::control_watches::WatchEngine::new(),
        })
    }

    async fn storage(&self) -> Option<Storage> {
        self.bridge.upgrade()?.storage_handle().await
    }

    /// The restart marker, then every pending decision and standing halt the
    /// journal has no row for yet — the attention state the previous process
    /// may not have journaled, whatever its last row's time says.
    pub async fn start(&self) {
        let Some(storage) = self.storage().await else { return };
        let _ = storage
            .insert_control_event(&NewControlEvent {
                session_id: JOURNAL_SESSION.into(),
                kind: "journal_restart".into(),
                detail: Some(
                    "journal restarted; events before this row may be missing; pending decisions and standing halts not yet journaled are re-listed below"
                        .into(),
                ),
                ..Default::default()
            })
            .await;
        self.reconcile_attention(&storage).await;
        // Plan 5c-2: every wake watch, from its own cursor — a row that
        // committed before the engine saw it, or that a late start missed, is
        // found here; a pending one is retried.
        if let Some(bridge) = self.bridge.upgrade() {
            self.watches.rearm_all(&bridge, &storage).await;
        }
    }

    /// The reconciliation alone (the start's second half), for a live gap.
    async fn reconcile_attention(&self, storage: &Storage) {
        let attention = |rows: Vec<AttentionRow>, kind: &str| {
            rows.into_iter()
                .map(|r| NewControlEvent {
                    session_id: r.session_id,
                    kind: kind.into(),
                    agent: Some(r.agent),
                    ref_id: r.ref_id,
                    detail: Some(cut(&r.detail)),
                    ..Default::default()
                })
                .collect::<Vec<_>>()
        };
        let mut rows = Vec::new();
        match storage.pending_decisions_open().await {
            Ok(decisions) => rows.extend(attention(decisions, "decision_pending")),
            Err(e) => tracing::warn!(?e, "control-event journal could not list pending decisions"),
        }
        match storage.standing_halts_open().await {
            Ok(halts) => rows.extend(attention(halts, "halt_declared")),
            Err(e) => tracing::warn!(?e, "control-event journal could not list standing halts"),
        }
        for row in rows {
            self.insert_once(&storage, row).await;
        }
    }

    /// Insert an attention row unless the journal already holds that
    /// identity (kind, session, `ref_id`). A row with no identity is inserted.
    async fn insert_once(&self, storage: &Storage, row: NewControlEvent) {
        if let Some(ref_id) = row.ref_id.as_deref() {
            match storage.control_event_exists(&row.kind, &row.session_id, ref_id).await {
                Ok(true) => return,
                Ok(false) => {}
                // Unsure is not "already there": a duplicate beats a lost row.
                Err(e) => tracing::warn!(?e, kind = %row.kind, "control-event journal identity check failed"),
            }
        }
        self.insert(storage, row).await;
    }

    /// The broadcast dropped `n` events before the task read them: the gap
    /// row, then the durable attention state the drop may have hidden is
    /// reconciled (deduped by identity) and every wake watch re-armed, so a
    /// default watcher asleep on a dropped `PendingChoice` or `AwaitingUser`
    /// still wakes without a restart (eyes-2, 5c-2 review).
    pub async fn gap(&self, n: u64) {
        let Some(storage) = self.storage().await else { return };
        self.insert(&storage, NewControlEvent {
            session_id: JOURNAL_SESSION.into(),
            kind: "journal_gap".into(),
            detail: Some(format!("{n} bridge events were dropped before the journal read them; pending decisions and standing halts not yet journaled are re-listed below")),
            ..Default::default()
        })
        .await;
        self.reconcile_attention(&storage).await;
        if let Some(bridge) = self.bridge.upgrade() {
            self.watches.rearm_all(&bridge, &storage).await;
        }
    }

    /// Journal one bridge event, or nothing.
    pub async fn record(&self, event: &SignalingEvent) {
        let Some(storage) = self.storage().await else { return };
        let row = match event {
            SignalingEvent::MessagePersisted { session_id, message_id } => {
                if !self.controller_present(&storage).await {
                    return;
                }
                let Ok(Some((origin, kind, slug))) = storage.message_journal_facts(*message_id).await else { return };
                if matches!(kind.as_str(), "tool_use" | "tool_result") {
                    return;
                }
                // A participant's row names it; the human's names "user"; a
                // host row names nobody.
                let agent = match origin.as_str() {
                    "participant" => slug,
                    "user" => Some("user".to_string()),
                    _ => None,
                };
                // A watch wake's notice is journaled as `watch_wake`, never as
                // a message: a wake must never arm another watch.
                let (source_session, source_slug, wake_notice) = match storage.delegated_message_source(*message_id).await {
                    Ok(Some((session, slug, request_id))) => (Some(session), Some(slug), request_id.starts_with("watch:")),
                    _ => (None, None, false),
                };
                NewControlEvent {
                    session_id: session_id.to_string(),
                    kind: if wake_notice { "watch_wake".into() } else { "message".into() },
                    agent,
                    ref_id: Some(message_id.to_string()),
                    detail: Some(kind),
                    source_session,
                    source_slug,
                }
            }
            SignalingEvent::PendingChoice(p) => {
                return self
                    .insert_once(&storage, NewControlEvent {
                        session_id: p.session_id.clone(),
                        kind: "decision_pending".into(),
                        agent: Some(p.agent.clone()),
                        ref_id: Some(p.choice_id.clone()),
                        detail: Some(cut(&p.question)),
                        ..Default::default()
                    })
                    .await;
            }
            SignalingEvent::ChoiceResolved { choice_id, picked } => {
                let Ok(Some(row)) = storage.get_tray_entry(choice_id).await else { return };
                NewControlEvent {
                    session_id: row.session_id,
                    kind: "decision_resolved".into(),
                    agent: Some(row.agent),
                    ref_id: Some(choice_id.clone()),
                    detail: Some(cut(picked)),
                    ..Default::default()
                }
            }
            SignalingEvent::AwaitingUser { session_id, agent, reason, halt_id } => {
                // The halt's identity is the id the EVENT carries, minted by
                // its declaration — never the slot's current value, which a
                // delayed event would borrow from a newer halt and then
                // suppress it (eyes-2 `d73ff458`), and never the millisecond
                // stamp two declarations can share (`04babed4`). An event
                // without one (the slot write failed) has no identity and is
                // kept as it came.
                return self
                    .insert_once(&storage, NewControlEvent {
                        session_id: session_id.clone(),
                        kind: "halt_declared".into(),
                        agent: Some(agent.clone()),
                        ref_id: halt_id.clone(),
                        detail: Some(cut(reason)),
                        ..Default::default()
                    })
                    .await;
            }
            SignalingEvent::HaltsCleared { session_id } => NewControlEvent {
                session_id: session_id.clone(),
                kind: "halt_cleared".into(),
                ..Default::default()
            },
            SignalingEvent::SessionClosed { session_id } => {
                self.forget_presence().await;
                NewControlEvent { session_id: session_id.clone(), kind: "session_closed".into(), ..Default::default() }
            }
            SignalingEvent::AgentHealth { session_id, agent, health } => NewControlEvent {
                session_id: session_id.clone(),
                kind: "health".into(),
                agent: Some(agent.clone()),
                detail: Some(health.clone()),
                ..Default::default()
            },
            SignalingEvent::SessionActivity { session_id, state, .. } => {
                if !self.controller_present(&storage).await {
                    return;
                }
                let mut last = self.last_activity.lock().await;
                let previous = match last.get(session_id) {
                    Some(previous) => Some(previous.clone()),
                    None => storage.last_journaled_activity(session_id).await.ok().flatten(),
                };
                if previous.as_deref() == Some(state.as_str()) {
                    return;
                }
                last.insert(session_id.clone(), state.clone());
                NewControlEvent {
                    session_id: session_id.clone(),
                    kind: "activity".into(),
                    detail: Some(state.clone()),
                    ..Default::default()
                }
            }
            SignalingEvent::GateUrgent { session_id, choice_id, agent, reason, .. } => NewControlEvent {
                session_id: session_id.clone(),
                kind: "gate_urgent".into(),
                agent: Some(agent.clone()),
                ref_id: Some(choice_id.clone()),
                detail: Some(cut(reason)),
                ..Default::default()
            },
            SignalingEvent::ParticipantChanged { session_id, agent, .. } => {
                self.forget_presence().await;
                NewControlEvent {
                    session_id: session_id.clone(),
                    kind: "participant_changed".into(),
                    agent: Some(agent.clone()),
                    ..Default::default()
                }
            }
            SignalingEvent::DocChanged { session_id } => NewControlEvent {
                session_id: session_id.clone(),
                kind: "doc_changed".into(),
                ..Default::default()
            },
            SignalingEvent::FindingsChanged { session_id } => NewControlEvent {
                session_id: session_id.clone(),
                kind: "findings_changed".into(),
                ..Default::default()
            },
            SignalingEvent::AgentAdvancePhase { session_id, agent, target } => NewControlEvent {
                session_id: session_id.clone(),
                kind: "phase_vote".into(),
                agent: Some(agent.clone()),
                detail: Some(target.clone()),
                ..Default::default()
            },
            SignalingEvent::ConfigurationChanged => {
                self.forget_presence().await;
                return;
            }
            SignalingEvent::AgentContext { .. }
            | SignalingEvent::SessionAttention { .. }
            | SignalingEvent::StageDelivered { .. }
            | SignalingEvent::StagedDeliveryDue { .. }
            | SignalingEvent::HaltAcked { .. }
            | SignalingEvent::StrayTurn { .. }
            | SignalingEvent::SessionCloseRequest { .. } => return,
        };
        self.insert(&storage, row).await;
    }

    async fn insert(&self, storage: &Storage, row: NewControlEvent) {
        let id = match storage.insert_control_event(&row).await {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(?e, kind = %row.kind, "control-event journal insert failed");
                return;
            }
        };
        if self.inserts.fetch_add(1, Ordering::Relaxed) % SWEEP_EVERY == SWEEP_EVERY - 1 {
            self.sweep(storage).await;
        }
        let event = crate::storage::ControlEvent {
            id,
            at: String::new(),
            session_id: row.session_id,
            kind: row.kind,
            agent: row.agent,
            ref_id: row.ref_id,
            detail: row.detail,
            source_session: row.source_session,
            source_slug: row.source_slug,
        };
        if let Some(bridge) = self.bridge.upgrade() {
            self.watches.on_event(&bridge, storage, &event).await;
        }
    }

    /// The wake engine, for tests.
    #[cfg(test)]
    pub fn watches(&self) -> &super::control_watches::WatchEngine {
        &self.watches
    }

    /// The retention sweep: rows older than [`CONTROL_EVENT_RETENTION_DAYS`]
    /// or past [`CONTROL_EVENT_MAX_ROWS`].
    pub async fn sweep(&self, storage: &Storage) {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(CONTROL_EVENT_RETENTION_DAYS))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        if let Err(e) = storage.purge_control_events(&cutoff, CONTROL_EVENT_MAX_ROWS).await {
            tracing::warn!(?e, "control-event journal sweep failed");
        }
    }

    async fn controller_present(&self, storage: &Storage) -> bool {
        let mut cached = self.presence.lock().await;
        if let Some((present, at)) = *cached {
            if at.elapsed() < PRESENCE_TTL {
                return present;
            }
        }
        // A read that fails keeps journaling: losing rows is the worse error.
        let present = storage.any_controller_present().await.unwrap_or(true);
        *cached = Some((present, std::time::Instant::now()));
        present
    }

    async fn forget_presence(&self) {
        *self.presence.lock().await = None;
        self.watches.forget_controller_sessions().await;
    }
}

impl SignalingBridge {
    /// Start the journal task: the restart marker and reconciliation, then
    /// every broadcast event until the bridge goes away. Started once by
    /// `main.rs` after storage is wired, like the scheduled-gate runner.
    /// Returns a receiver that resolves once the start — the marker, the
    /// reconciliation and the watch re-arm — has run, so the boot respawn can
    /// read the watches' pending state after it, not beside it.
    pub fn spawn_control_event_journal(self: &Arc<Self>) -> tokio::sync::oneshot::Receiver<()> {
        let journal = ControlEventJournal::new(self);
        let mut events = self.subscribe();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            journal.start().await;
            let _ = ready_tx.send(());
            loop {
                match events.recv().await {
                    Ok(event) => journal.record(&event).await,
                    Err(broadcast::error::RecvError::Lagged(n)) => journal.gap(n).await,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
                if journal.bridge.upgrade().is_none() {
                    break;
                }
            }
        });
        ready_rx
    }

    /// A journal handle driven by the test instead of the task.
    #[cfg(test)]
    pub fn control_event_journal_for_tests(self: &Arc<Self>) -> Arc<ControlEventJournal> {
        ControlEventJournal::new(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fixture() -> (Arc<SignalingBridge>, Storage, Arc<ControlEventJournal>) {
        let bridge = SignalingBridge::new();
        let storage = Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("t", "target", None).await.unwrap();
        storage.ensure_session_roster("t", 2).await.unwrap();
        storage.create_session("c", "controller", None).await.unwrap();
        storage.ensure_session_roster("c", 2).await.unwrap();
        sqlx::query("UPDATE session_participants SET capabilities = '[\"control_bot_hq\"]' WHERE session_id = 'c' AND slug = 'hands'")
            .execute(storage.pool()).await.unwrap();
        let journal = bridge.control_event_journal_for_tests();
        (bridge, storage, journal)
    }

    async fn rows(storage: &Storage) -> Vec<crate::storage::ControlEvent> {
        storage.control_events_page(0, None, None, 100).await.unwrap().events
    }

    /// Every journaled variant lands as its kind with the fields a controller
    /// filters on; tool rows and the plumbing variants land nowhere; a
    /// delegated message names the controller that sent it; secrets in a
    /// detail are redacted and the detail is cut.
    #[tokio::test]
    async fn each_bridge_event_becomes_one_row_or_none() {
        let (_bridge, storage, journal) = fixture().await;
        let text = storage.post_to_channel("t", "participant", Some("hands"), "text", "pushed", None).await.unwrap().message_id();
        let tool = storage.post_to_channel("t", "participant", Some("hands"), "tool_use", "{}", None).await.unwrap().message_id();
        let result = storage.post_to_channel("t", "participant", Some("hands"), "tool_result", "{}", None).await.unwrap().message_id();
        let notice = storage.post_to_channel("t", "system", None, "system_notice", "[System: hi]", None).await.unwrap().message_id();
        let delegated = storage.post_to_channel("t", "system", None, "system_notice", "Delegated message from hands of c: go", None).await.unwrap().message_id();
        let controller_id: i64 = sqlx::query_scalar("SELECT id FROM session_participants WHERE session_id = 'c' AND slug = 'hands'").fetch_one(storage.pool()).await.unwrap();
        sqlx::query("INSERT INTO delegated_messages (source_session, source_participant, request_id, target_session, content_sha256, message_id) VALUES ('c', ?, 'r1', 't', 'x', ?)")
            .bind(controller_id).bind(delegated).execute(storage.pool()).await.unwrap();
        for id in [text, tool, result, notice, delegated] {
            journal.record(&SignalingEvent::MessagePersisted { session_id: Arc::from("t"), message_id: id }).await;
        }
        let yes_no = ["yes".to_string(), "no".to_string()];
        storage.insert_tray_entry("t", "q1", "hands", crate::storage::QuestionKind::Choice, "Deploy the token ghp_abcdefghijklmnopqrstuvwxyz0123456789 now?", Some(&yes_no), None, None).await.unwrap();
        journal.record(&SignalingEvent::PendingChoice(PendingChoice { choice_id: "q1".into(), session_id: "t".into(), agent: "hands".into(), question: format!("Deploy {}?", "x".repeat(400)), options: yes_no.to_vec(), approval: None })).await;
        journal.record(&SignalingEvent::ChoiceResolved { choice_id: "q1".into(), picked: "yes".into() }).await;
        journal.record(&SignalingEvent::ChoiceResolved { choice_id: "absent".into(), picked: "yes".into() }).await;
        journal.record(&SignalingEvent::AwaitingUser { session_id: "t".into(), agent: "eyes".into(), reason: "CI".into(), halt_id: Some("2026-10-26T00:00:00.000Z".into()) }).await;
        journal.record(&SignalingEvent::HaltsCleared { session_id: "t".into() }).await;
        journal.record(&SignalingEvent::AgentHealth { session_id: "t".into(), agent: "hands".into(), health: "stalled".into() }).await;
        journal.record(&SignalingEvent::GateUrgent { session_id: "t".into(), choice_id: "g1".into(), agent: "hands".into(), reason: "prod".into(), command: "rm".into(), under_review: false }).await;
        journal.record(&SignalingEvent::ParticipantChanged { session_id: "t".into(), agent: "eyes".into(), context_reset: true }).await;
        journal.record(&SignalingEvent::DocChanged { session_id: "t".into() }).await;
        journal.record(&SignalingEvent::FindingsChanged { session_id: "t".into() }).await;
        journal.record(&SignalingEvent::AgentAdvancePhase { session_id: "t".into(), agent: "hands".into(), target: "Verify".into() }).await;
        journal.record(&SignalingEvent::SessionClosed { session_id: "t".into() }).await;
        for quiet in [
            SignalingEvent::ConfigurationChanged,
            SignalingEvent::AgentContext { session_id: "t".into(), agent: "hands".into(), used_tokens: 1, context_window: 2 },
            SignalingEvent::SessionAttention { session_id: "t".into(), state: None },
            SignalingEvent::StageDelivered { session_id: "t".into() },
            SignalingEvent::StagedDeliveryDue { session_id: "t".into() },
            SignalingEvent::HaltAcked { session_id: "t".into(), agent: "hands".into(), incarnation: None },
            SignalingEvent::SessionCloseRequest { session_id: "t".into(), agent: "hands".into(), archive: false },
        ] {
            journal.record(&quiet).await;
        }
        let rows_now = rows(&storage).await;
        let rows = &rows_now;
        let kinds: Vec<&str> = rows.iter().map(|r| r.kind.as_str()).collect();
        assert_eq!(kinds, [
            "message", "message", "message", "decision_pending", "decision_resolved", "halt_declared", "halt_cleared",
            "health", "gate_urgent", "participant_changed", "doc_changed", "findings_changed", "phase_vote", "session_closed",
        ], "{rows:#?}");
        assert_eq!((rows[0].agent.as_deref(), rows[0].ref_id.as_deref(), rows[0].detail.as_deref()), (Some("hands"), Some(text.to_string().as_str()), Some("text")));
        assert_eq!((rows[1].agent.as_deref(), rows[1].detail.as_deref(), rows[1].source_session.as_deref()), (None, Some("system_notice"), None), "a host row names nobody");
        assert_eq!((rows[2].source_session.as_deref(), rows[2].source_slug.as_deref()), (Some("c"), Some("hands")), "a delegated message names its controller");
        assert!(rows.iter().all(|r| r.session_id == "t"));
        let pending = &rows[3];
        assert_eq!((pending.agent.as_deref(), pending.ref_id.as_deref()), (Some("hands"), Some("q1")));
        assert_eq!(pending.detail.as_ref().unwrap().len(), DETAIL_BYTES, "detail is cut");
        assert_eq!((rows[4].ref_id.as_deref(), rows[4].detail.as_deref(), rows[4].agent.as_deref()), (Some("q1"), Some("yes"), Some("hands")));
        assert_eq!((rows[5].agent.as_deref(), rows[5].detail.as_deref()), (Some("eyes"), Some("CI")));
        assert_eq!(rows[7].detail.as_deref(), Some("stalled"));
        assert_eq!((rows[8].ref_id.as_deref(), rows[8].detail.as_deref()), (Some("g1"), Some("prod")));
        assert_eq!(rows[12].detail.as_deref(), Some("Verify"));
        // A redacted detail never carries the secret.
        journal.record(&SignalingEvent::AwaitingUser { session_id: "t".into(), agent: "hands".into(), reason: "token ghp_abcdefghijklmnopqrstuvwxyz0123456789 leaked".into(), halt_id: None }).await;
        let last = super::tests::rows(&storage).await.pop().unwrap();
        assert!(!last.detail.as_deref().unwrap().contains("ghp_abcdefghijklmnopqrstuvwxyz0123456789"), "{last:?}");
    }

    /// A repeated activity state is journaled once — across the task's own
    /// memory and across a restart, through the table.
    #[tokio::test]
    async fn activity_is_journaled_on_change_only() {
        let (bridge, storage, journal) = fixture().await;
        for state in ["idle", "idle", "working", "working", "idle"] {
            journal.record(&SignalingEvent::SessionActivity { session_id: "t".into(), state: state.into(), slot0_busy: false, slot1_busy: false, busy_slots: vec![] }).await;
        }
        let journaled = rows(&storage).await;
        let states: Vec<Option<&str>> = journaled.iter().map(|r| r.detail.as_deref()).collect();
        assert_eq!(states, [Some("idle"), Some("working"), Some("idle")]);
        // A fresh journal (a restart) reads the last state from the table.
        let fresh = bridge.control_event_journal_for_tests();
        fresh.record(&SignalingEvent::SessionActivity { session_id: "t".into(), state: "idle".into(), slot0_busy: false, slot1_busy: false, busy_slots: vec![] }).await;
        fresh.record(&SignalingEvent::SessionActivity { session_id: "t".into(), state: "working".into(), slot0_busy: true, slot1_busy: false, busy_slots: vec![0] }).await;
        assert_eq!(rows(&storage).await.len(), 4);
        // Another session's states are their own sequence.
        fresh.record(&SignalingEvent::SessionActivity { session_id: "c".into(), state: "idle".into(), slot0_busy: false, slot1_busy: false, busy_slots: vec![] }).await;
        assert_eq!(rows(&storage).await.len(), 5);
    }

    /// Without a controller anywhere, the chatty kinds are not journaled and
    /// the attention kinds still are; the presence answer is cached and
    /// forgotten on a roster change.
    #[tokio::test]
    async fn chatty_kinds_wait_for_a_controller_to_exist() {
        let bridge = SignalingBridge::new();
        let storage = Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        storage.create_session("t", "target", None).await.unwrap();
        storage.ensure_session_roster("t", 2).await.unwrap();
        let journal = bridge.control_event_journal_for_tests();
        let text = storage.post_to_channel("t", "participant", Some("hands"), "text", "pushed", None).await.unwrap().message_id();
        journal.record(&SignalingEvent::MessagePersisted { session_id: Arc::from("t"), message_id: text }).await;
        journal.record(&SignalingEvent::SessionActivity { session_id: "t".into(), state: "idle".into(), slot0_busy: false, slot1_busy: false, busy_slots: vec![] }).await;
        journal.record(&SignalingEvent::AwaitingUser { session_id: "t".into(), agent: "hands".into(), reason: "CI".into(), halt_id: None }).await;
        assert_eq!(rows(&storage).await.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>(), ["halt_declared"]);
        // A controller appears: the cached answer stands until the roster
        // change that announces it, then messages are journaled.
        sqlx::query("UPDATE session_participants SET capabilities = '[\"control_bot_hq\"]' WHERE session_id = 't' AND slug = 'eyes'").execute(storage.pool()).await.unwrap();
        journal.record(&SignalingEvent::MessagePersisted { session_id: Arc::from("t"), message_id: text }).await;
        assert_eq!(rows(&storage).await.len(), 1, "cached: not yet");
        journal.record(&SignalingEvent::ParticipantChanged { session_id: "t".into(), agent: "eyes".into(), context_reset: true }).await;
        journal.record(&SignalingEvent::MessagePersisted { session_id: Arc::from("t"), message_id: text }).await;
        assert_eq!(rows(&storage).await.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>(), ["halt_declared", "participant_changed", "message"]);
    }

    /// The start of the task: a restart marker, then every pending decision
    /// and standing halt the journal has no row for — deduped by identity,
    /// never by the newest row's time (eyes-2 `5c-1`): a catch-up the previous
    /// process died halfway through, or an older decision beside a newer
    /// unrelated row, is still reconciled; a decision whose `PendingChoice`
    /// was buffered during the catch-up lands once. A lagged receiver leaves a
    /// gap row with the count.
    #[tokio::test]
    async fn start_marks_the_restart_and_reconciles_attention_state_by_identity() {
        let (_bridge, storage, journal) = fixture().await;
        let yes_no = ["yes".to_string(), "no".to_string()];
        storage.insert_tray_entry("t", "q1", "hands", crate::storage::QuestionKind::Choice, "Deploy?", Some(&yes_no), None, None).await.unwrap();
        let declared_at = storage.declare_session_halt_stamped("c", "hands", "waiting for events").await.unwrap().unwrap();
        journal.start().await;
        let rows1 = rows(&storage).await;
        assert_eq!(rows1.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>(), ["journal_restart", "decision_pending", "halt_declared"], "{rows1:#?}");
        assert_eq!(rows1[0].session_id, JOURNAL_SESSION);
        assert!(rows1[0].detail.as_deref().unwrap().contains("may be missing"));
        assert_eq!((rows1[1].session_id.as_str(), rows1[1].ref_id.as_deref(), rows1[1].detail.as_deref()), ("t", Some("q1"), Some("Deploy?")));
        assert_eq!((rows1[2].session_id.as_str(), rows1[2].agent.as_deref(), rows1[2].ref_id.as_deref(), rows1[2].detail.as_deref()), ("c", Some("hands"), Some(declared_at.as_str()), Some("waiting for events")));
        // An interrupted catch-up: the previous process wrote its marker and
        // a newer unrelated row, then died before reaching q2 — which is older
        // than that row. A time cutoff would skip q2 for good; identity does not.
        storage.insert_tray_entry("t", "q2", "eyes", crate::storage::QuestionKind::Choice, "Merge?", Some(&yes_no), None, None).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        storage.insert_control_event(&NewControlEvent { session_id: JOURNAL_SESSION.into(), kind: "journal_restart".into(), ..Default::default() }).await.unwrap();
        journal.record(&SignalingEvent::AgentHealth { session_id: "t".into(), agent: "hands".into(), health: "running".into() }).await;
        journal.start().await;
        let rows2 = rows(&storage).await;
        assert_eq!(rows2[3..].iter().map(|r| r.kind.as_str()).collect::<Vec<_>>(), ["journal_restart", "health", "journal_restart", "decision_pending"], "{rows2:#?}");
        assert_eq!(rows2[6].ref_id.as_deref(), Some("q2"), "the missed older decision is reconciled; q1 and the halt are not repeated");
        // A buffered PendingChoice for a decision the catch-up already wrote
        // lands once; a new decision lands as usual; an answered decision is
        // not re-listed by a later start.
        journal.record(&SignalingEvent::PendingChoice(PendingChoice { choice_id: "q2".into(), session_id: "t".into(), agent: "eyes".into(), question: "Merge?".into(), options: yes_no.to_vec(), approval: None })).await;
        assert_eq!(rows(&storage).await.len(), 7, "no duplicate");
        storage.insert_tray_entry("t", "q3", "hands", crate::storage::QuestionKind::Choice, "Tag?", Some(&yes_no), None, None).await.unwrap();
        journal.record(&SignalingEvent::PendingChoice(PendingChoice { choice_id: "q3".into(), session_id: "t".into(), agent: "hands".into(), question: "Tag?".into(), options: yes_no.to_vec(), approval: None })).await;
        sqlx::query("UPDATE session_tray SET status = 'answered' WHERE choice_id = 'q1'").execute(storage.pool()).await.unwrap();
        journal.start().await;
        let rows3 = rows(&storage).await;
        assert_eq!(rows3[7..].iter().map(|r| r.kind.as_str()).collect::<Vec<_>>(), ["decision_pending", "journal_restart"], "{rows3:#?}");
        // The live event for the standing halt carries the id its declaration
        // minted and dedupes against the reconciled row; a newer halt is a new
        // identity even in the same millisecond (eyes-2 `04babed4`: no sleep
        // here). A DELAYED event for the old halt, arriving after the newer
        // one replaced the slot, keeps its own identity and reason — it never
        // borrows the newer one (`d73ff458`), so the newer halt's own event is
        // not suppressed and its row is right.
        journal.record(&SignalingEvent::AwaitingUser { session_id: "c".into(), agent: "hands".into(), reason: "waiting for events".into(), halt_id: Some(declared_at.clone()) }).await;
        assert_eq!(rows(&storage).await.len(), 9, "the standing halt is already journaled");
        let newer = storage.declare_session_halt_stamped("c", "hands", "waiting again").await.unwrap().unwrap();
        assert_ne!(newer, declared_at);
        // The old halt's event arrives late, after the slot moved on.
        journal.record(&SignalingEvent::AwaitingUser { session_id: "c".into(), agent: "hands".into(), reason: "waiting for events".into(), halt_id: Some(declared_at.clone()) }).await;
        assert_eq!(rows(&storage).await.len(), 9, "the delayed old event is the already-journaled old halt");
        journal.record(&SignalingEvent::AwaitingUser { session_id: "c".into(), agent: "hands".into(), reason: "waiting again".into(), halt_id: Some(newer.clone()) }).await;
        let last = rows(&storage).await.pop().unwrap();
        assert_eq!((last.kind.as_str(), last.detail.as_deref(), last.ref_id.as_deref()), ("halt_declared", Some("waiting again"), Some(newer.as_str())));
        // An event whose slot write failed has no identity: kept as it came, twice if it comes twice.
        journal.record(&SignalingEvent::AwaitingUser { session_id: "c".into(), agent: "hands".into(), reason: "unrecorded".into(), halt_id: None }).await;
        journal.record(&SignalingEvent::AwaitingUser { session_id: "c".into(), agent: "hands".into(), reason: "unrecorded".into(), halt_id: None }).await;
        assert_eq!(rows(&storage).await.len(), 12);
        journal.gap(17).await;
        let last = rows(&storage).await.pop().unwrap();
        assert_eq!((last.kind.as_str(), last.session_id.as_str()), ("journal_gap", JOURNAL_SESSION));
        assert!(last.detail.as_deref().unwrap().starts_with("17 bridge events were dropped"));
    }

    /// The spawned task consumes the bridge's broadcast: a notify on the bridge
    /// becomes a row without any call from the test.
    #[tokio::test]
    async fn the_task_journals_what_the_bridge_broadcasts() {
        let (bridge, storage, _journal) = fixture().await;
        let ready = bridge.spawn_control_event_journal();
        ready.await.expect("the start ran");
        bridge.notify_agent_health("t".to_string(), "hands", "stalled");
        let mut seen = false;
        for _ in 0..40 {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            if rows(&storage).await.iter().any(|r| r.kind == "health" && r.detail.as_deref() == Some("stalled")) {
                seen = true;
                break;
            }
        }
        assert!(seen, "the task journals the broadcast");
        assert_eq!(rows(&storage).await[0].kind, "journal_restart", "the task started with its marker");
    }
}
