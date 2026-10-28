//! Watches and their wake (Plan 5c-2). A controller subscribes to the
//! control-event journal (`control_watches`, 0113): which sessions, which
//! kinds, and whether a matching event may WAKE it. The journal task hands
//! every row it writes to [`WatchEngine::on_event`]; a watch whose filters
//! match is ARMED with that row's id (once — the first unacknowledged match
//! stays the one), and the engine tries to fire.
//!
//! **What a wake is.** `fire_temporary_halt`'s discipline, not a `Summon` (a
//! summons queues behind a declared halt, and an AFK controller waits halted):
//! a system row in the controller's session naming the watch, the pending
//! event and the `events` call that reads it; the controller's own declared
//! halt cleared — by `halt_id`, so a newer halt is never cleared by an older
//! decision; the awaiting flag dropped; and the ring released with the
//! controller mentioned, which deals it the next turn. The notice is linked in
//! `delegated_messages` under `watch:<watch>:<event>`, so the journal files
//! it as `watch_wake` — a kind no watch can arm.
//!
//! **What stops a wake, rechecked at the release boundary.** The watch must
//! still hold that pending event and not have fired for it (the one-shot claim
//! in storage — two triggers fire one notice); the controller must still be an
//! enabled participant of an open session whose role grants control (a
//! confirmed loss deletes the watch and journals `watch_dropped`; a read
//! failure keeps it pending); the session must have a ring; the tracker must
//! not be paused (the user's Pause is never released by a wake) and must not
//! be busy (a working controller reads its events itself); no approval gate
//! may be pending in the controller's session. A refused fire stays pending
//! and is retried when the controller's own session changes (its halt is
//! declared, a gate resolves, activity drops) and when its ring registers; a
//! boot respawns a dormant controller whose pending event is an attention
//! kind, as it respawns temporary halts.
//!
//! **Loop guards.** A watch never matches its own session; `targets: null`
//! means every other session WITHOUT a controller (two controllers watching
//! "everything" never see each other); `kinds: null` means the attention kinds
//! (`decision_pending`, `halt_declared`, `gate_urgent`, `health` when a runtime is `dead`,
//! `session_closed`) — `message` and `activity` wake only when named; a row the
//! controller itself caused (`source_session`/`source_slug`) never arms its
//! watch; a wake's own notice is `watch_wake` and arms nothing; and a watch
//! fires at most once per acknowledgement (`ack`, explicit and per watch — a
//! read never acknowledges). A controller that names another controller's
//! session AND `message`/`activity` can still be woken by that controller's
//! replies; the per-ack bound keeps it to one wake per read, and that is a
//! choice the controller made.

use super::*;
use crate::storage::{default_kind_matches, ControlActor, ControlEvent, ControlWatch, NewControlEvent, MAX_WATCHES_PER_CONTROLLER, WATCHABLE_KINDS};
use anyhow::Context;

/// How long the controller-session list is trusted.
const CONTROLLER_SESSIONS_TTL: std::time::Duration = std::time::Duration::from_secs(30);
/// One page of the re-arm scan; the scan pages to the journal's end.
const REARM_PAGE: usize = 500;
/// A watch fires at most once per this interval — a backstop that rate-limits
/// (not terminates) a loop two controllers build by naming each other's
/// sessions with `message`/`activity`; the pending event stays and the
/// periodic retry fires it after the gap.
pub const WAKE_MIN_GAP: std::time::Duration = std::time::Duration::from_secs(60);

/// Whether `event` is one `watch` subscribes to. Pure; `controller_sessions`
/// are the open sessions holding a controller, excluded from a `targets: null`
/// watch.
pub fn watch_matches(watch: &ControlWatch, event: &ControlEvent, controller_sessions: &[String]) -> bool {
    if event.session_id == watch.session_id {
        return false;
    }
    if event.kind == "watch_wake" {
        return false;
    }
    if event.source_session.as_deref() == Some(watch.session_id.as_str())
        && event.source_slug.as_deref() == Some(watch.slug.as_str())
    {
        return false;
    }
    let target_ok = match &watch.targets {
        Some(targets) => targets.iter().any(|t| *t == event.session_id),
        None => !controller_sessions.iter().any(|s| *s == event.session_id),
    };
    if !target_ok {
        return false;
    }
    match &watch.kinds {
        Some(kinds) => kinds.iter().any(|k| *k == event.kind),
        None => default_kind_matches(&event.kind, event.detail.as_deref()),
    }
}

/// The wake engine the journal task owns. Holds only the controller-session
/// cache and the test barrier; the bridge it fires through is passed in, so
/// the bridge's own `&self` methods can drive it too.
#[derive(Default)]
pub struct WatchEngine {
    controller_sessions: Mutex<Option<(Vec<String>, std::time::Instant)>>,
    /// Runs at one point of the delivery, so a test can change the world
    /// there: after the notice and before the release-boundary recheck, or
    /// after that recheck and before the halt clear.
    #[cfg(test)]
    barrier: Mutex<Option<(BarrierPoint, Barrier)>>,
}

#[cfg(test)]
type Barrier = Box<dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum BarrierPoint {
    AfterNotice,
    BeforeClear,
}

/// Why a fire did not happen now — pending stays, or the watch is gone.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum FireOutcome {
    /// Handed to the ring, which acknowledges by `watch_wake_consumed`.
    Released,
    NothingPending,
    AlreadyFired,
    Dropped(String),
    Deferred(&'static str),
}

/// The halt a wake decided against: by id, or — declared by a build before
/// `halt_id` existed — by its whole observed slot; or none.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DecidedHalt {
    None,
    Id(String),
    Legacy { by: String, reason: String, at: String },
}

/// What the release boundary found when it looked again.
enum Admission {
    Admit { actor: ControlActor, halt: DecidedHalt, attempt: Option<String> },
    Defer(&'static str),
    Drop(String),
}

impl WatchEngine {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    async fn barrier_step(&self, point: BarrierPoint) {
        let step = self.barrier.lock().await.as_ref().filter(|(at, _)| *at == point).map(|(_, barrier)| barrier());
        if let Some(step) = step {
            step.await;
        }
    }

    #[cfg(not(test))]
    async fn barrier_step(&self, _point: BarrierPoint) {}

    fn min_gap(bridge: &SignalingBridge) -> std::time::Duration {
        std::time::Duration::from_secs(bridge.watch_min_gap_secs.load(Ordering::Relaxed))
    }

    async fn controller_sessions(&self, storage: &Storage) -> Vec<String> {
        let mut cached = self.controller_sessions.lock().await;
        if let Some((sessions, at)) = &*cached {
            if at.elapsed() < CONTROLLER_SESSIONS_TTL {
                return sessions.clone();
            }
        }
        match storage.controller_sessions_open().await {
            Ok(sessions) => {
                *cached = Some((sessions.clone(), std::time::Instant::now()));
                sessions
            }
            // Unknown: keep the last answer, or exclude nothing — a wake too
            // many is cheaper than a missed attention state.
            Err(e) => {
                tracing::warn!(?e, "could not list the controller sessions");
                cached.as_ref().map(|(s, _)| s.clone()).unwrap_or_default()
            }
        }
    }

    pub async fn forget_controller_sessions(&self) {
        *self.controller_sessions.lock().await = None;
    }

    /// A row the journal just wrote: arm the watches it matches, then retry
    /// the pending watches of the session it is about (the controller's own
    /// state changed).
    pub async fn on_event(&self, bridge: &SignalingBridge, storage: &Storage, event: &ControlEvent) {
        let armed = match storage.armed_control_watches().await {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(?e, "could not list the armed watches");
                return;
            }
        };
        if !armed.is_empty() {
            let controllers = self.controller_sessions(storage).await;
            for watch in armed.iter().filter(|w| w.pending_since.is_none()) {
                if watch_matches(watch, event, &controllers) {
                    match storage.arm_control_watch(watch.id, event.id, &event.kind).await {
                        Ok(true) => {
                            self.try_fire(bridge, storage, watch.id).await;
                        }
                        Ok(false) => {}
                        Err(e) => tracing::warn!(?e, watch = watch.id, "could not arm a watch"),
                    }
                }
            }
        }
        if event.session_id != crate::storage::JOURNAL_SESSION {
            self.retry_pending_for_session(bridge, storage, &event.session_id).await;
        }
    }

    /// The controller's session changed (or its ring came up): try its pending
    /// watches again.
    pub async fn retry_pending_for_session(&self, bridge: &SignalingBridge, storage: &Storage, session_id: &str) {
        match storage.pending_control_watches_for_session(session_id).await {
            Ok(watches) => {
                for watch in watches {
                    self.try_fire(bridge, storage, watch.id).await;
                }
            }
            Err(e) => tracing::warn!(?e, session_id, "could not list the session's pending watches"),
        }
    }

    /// Every wake-enabled watch, from its own cursor: an idle one is re-armed
    /// against the journal (a row that committed before the engine saw it —
    /// a crash in between, a journal task that started late — is found
    /// here), a pending one is retried. The journal runs this at start and
    /// after a gap; the scheduler tick runs it periodically, which is what
    /// fires a wake deferred for busyness, a gate or the rate limit once
    /// nothing else prompts it.
    pub async fn rearm_all(&self, bridge: &SignalingBridge, storage: &Storage) {
        let watches = match storage.wake_control_watches().await {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(?e, "could not list the wake watches");
                return;
            }
        };
        for watch in watches {
            if watch.pending_since.is_some() {
                self.try_fire(bridge, storage, watch.id).await;
            } else if let Err(e) = self.rearm_from_cursor(bridge, storage, watch.id).await {
                tracing::warn!(?e, watch = watch.id, "re-arm failed");
            }
        }
    }

    /// Re-arm from the first matching event after the cursor — paging to the
    /// journal's end, never a bounded window (eyes-2 `9c503a55`) — then try to
    /// fire.
    pub async fn rearm_from_cursor(&self, bridge: &SignalingBridge, storage: &Storage, watch_id: i64) -> Result<Option<ControlWatch>> {
        let Some(watch) = storage.control_watch_by_id(watch_id).await? else { return Ok(None) };
        if !watch.wake || watch.pending_since.is_some() {
            return Ok(Some(watch));
        }
        let controllers = self.controller_sessions(storage).await;
        let mut from = watch.cursor;
        loop {
            let page = storage.control_events_after(from, REARM_PAGE).await?;
            if let Some(first) = page.iter().find(|e| watch_matches(&watch, e, &controllers)) {
                if storage.arm_control_watch(watch.id, first.id, &first.kind).await? {
                    self.try_fire(bridge, storage, watch.id).await;
                }
                break;
            }
            match page.last() {
                Some(last) if page.len() == REARM_PAGE => from = last.id,
                _ => break,
            }
        }
        storage.control_watch_by_id(watch_id).await
    }

    /// Everything the release boundary must find true, read fresh: the watch
    /// as claimed (same pending event, same claim, still wake-enabled), the
    /// grant, a ring, no Pause, no live turn, no pending gate, and the halt
    /// slot by id. Any failure to READ is a deferral, never an admission.
    async fn admission(&self, bridge: &SignalingBridge, storage: &Storage, watch: &ControlWatch, pending: i64, claimed: bool) -> Admission {
        let current = match storage.control_watch_by_id(watch.id).await {
            Ok(Some(w)) => w,
            Ok(None) => return Admission::Drop("the watch was removed".into()),
            Err(_) => return Admission::Defer("watch read failed"),
        };
        if !current.wake || current.pending_since != Some(pending) {
            return Admission::Defer("the watch changed under the wake");
        }
        if claimed && (current.woke_for != Some(pending) || current.wake_delivered_at.is_some()) {
            return Admission::Defer("the watch changed under the wake");
        }
        let actor = match storage.controller_actor(&watch.session_id, &watch.slug).await {
            Ok(actor) => actor,
            Err(e) => {
                let text = e.to_string();
                if text.contains("not granted") || text.contains("revoked") || text.contains("no longer an enabled participant") {
                    return Admission::Drop(text);
                }
                return Admission::Defer("controller check failed");
            }
        };
        if !bridge.session_sequencer.lock().await.contains_key(&watch.session_id) {
            return Admission::Defer("no ring");
        }
        if let Some(tracker) = bridge.session_activity.lock().await.get(&watch.session_id).and_then(Weak::upgrade) {
            if tracker.holds_wakes() {
                return Admission::Defer("paused");
            }
            if matches!(tracker.current(), crate::core::activity::SessionActivity::Busy | crate::core::activity::SessionActivity::Cancelling) {
                return Admission::Defer("busy");
            }
        }
        match storage.pending_gate_ids(&watch.session_id).await {
            Ok(gates) if !gates.is_empty() => return Admission::Defer("gate pending"),
            Ok(_) => {}
            Err(_) => return Admission::Defer("gate read failed"),
        }
        let halt_id = match storage.session_halt_id(&watch.session_id).await {
            Ok(id) => id,
            Err(_) => return Admission::Defer("halt read failed"),
        };
        let standing_halt = match storage.session_halt(&watch.session_id).await {
            Ok(h) => h,
            Err(_) => return Admission::Defer("halt read failed"),
        };
        let halt = match (halt_id, standing_halt) {
            (Some(id), _) => DecidedHalt::Id(id),
            (None, Some((by, reason, at))) => DecidedHalt::Legacy { by, reason, at },
            (None, None) => DecidedHalt::None,
        };
        Admission::Admit { actor, halt, attempt: current.wake_attempt_at }
    }

    /// Drop the watch on a confirmed loss of control, journaling why.
    async fn drop_watch(&self, storage: &Storage, watch: &ControlWatch, why: &str) {
        let _ = storage.delete_control_watch_by_id(watch.id).await;
        let _ = storage
            .insert_control_event(&NewControlEvent {
                session_id: watch.session_id.clone(),
                kind: "watch_dropped".into(),
                agent: Some(watch.slug.clone()),
                ref_id: Some(watch.request_id.clone()),
                detail: Some(format!("watch dropped: {why}")),
                source_session: Some(watch.session_id.clone()),
                source_slug: Some(watch.slug.clone()),
            })
            .await;
        tracing::info!(watch = watch.id, %why, "watch dropped on a confirmed loss of control");
    }

    /// The wake: claim (or resume an undelivered claim), write the notice
    /// once, recheck everything at the release boundary, then release.
    pub async fn try_fire(&self, bridge: &SignalingBridge, storage: &Storage, watch_id: i64) -> FireOutcome {
        let watch = match storage.control_watch_by_id(watch_id).await {
            Ok(Some(w)) => w,
            Ok(None) => return FireOutcome::NothingPending,
            Err(_) => return FireOutcome::Deferred("watch read failed"),
        };
        let Some(pending) = watch.pending_since else { return FireOutcome::NothingPending };
        if !watch.wake {
            return FireOutcome::NothingPending;
        }
        // The grant first, so a revoked controller's watch is dropped at any
        // retry: a confirmed loss drops the watch; a read failure waits.
        if let Err(e) = storage.controller_actor(&watch.session_id, &watch.slug).await {
            let text = e.to_string();
            if text.contains("not granted") || text.contains("revoked") || text.contains("no longer an enabled participant") {
                self.drop_watch(storage, &watch, &text).await;
                return FireOutcome::Dropped(text);
            }
            return FireOutcome::Deferred("controller check failed");
        }
        let resuming = watch.woke_for == Some(pending);
        if resuming && watch.wake_delivered_at.is_some() {
            return FireOutcome::AlreadyFired;
        }
        // **The pending event is re-validated against the watch's filter
        // before a claim** (feedback #144). A persisted pending is what an
        // earlier build armed: a default watch left on `health: stalled` must
        // not wake now that the default is dead-only. An event the retention
        // swept is judged by the kind kept on the watch — a swept default
        // `health` is stale by construction (its detail is gone and it is
        // old news). Disarmed without moving the ack cursor; the re-arm from
        // the cursor then finds the next event that does match. A claimed
        // wake is in flight and is left to its own path.
        if !resuming {
            let still_matches = match storage.control_event_by_id(pending).await {
                Ok(Some(event)) => {
                    let controllers = self.controller_sessions(storage).await;
                    watch_matches(&watch, &event, &controllers)
                }
                Ok(None) => !(watch.kinds.is_none() && watch.pending_kind.as_deref() == Some("health")),
                Err(_) => return FireOutcome::Deferred("pending event read failed"),
            };
            if !still_matches {
                match storage.disarm_control_watch(watch.id, pending).await {
                    Ok(true) => {
                        tracing::info!(watch = watch.id, pending, "watch pending event no longer matches its filter; disarmed");
                        // Boxed: the re-arm fires the next match through this
                        // same function, a recursion the future's size cannot
                        // hold inline. It ends: a re-arm arms only an event
                        // that matches, and a matching pending never disarms.
                        if let Err(e) = Box::pin(self.rearm_from_cursor(bridge, storage, watch.id)).await {
                            tracing::warn!(?e, watch = watch.id, "re-arm after disarming failed");
                        }
                    }
                    Ok(false) => {}
                    Err(_) => return FireOutcome::Deferred("disarm failed"),
                }
                return FireOutcome::NothingPending;
            }
        }
        if resuming {
            // The ring's own receipt outlives a crash between its deal and
            // the delivery marker: a notice already dealt to the controller
            // is a delivered wake. An unreadable receipt is unknown — it
            // defers, it never authorizes another release (eyes-2 `4fe7a60d`).
            match storage.wake_notice_message_id(&watch, pending).await {
                Ok(Some(notice)) => match storage.wake_notice_delivered(watch.participant_id, notice).await {
                    Ok(true) => {
                        let _ = storage.mark_control_watch_delivered(watch.id, pending).await;
                        return FireOutcome::AlreadyFired;
                    }
                    Ok(false) => {}
                    Err(_) => return FireOutcome::Deferred("receipt read failed"),
                },
                Ok(None) => {}
                Err(_) => return FireOutcome::Deferred("notice lookup failed"),
            }
            // One attempt at a time: the lease (eyes-2 `9912471a`).
            let cutoff = (chrono::Utc::now() - chrono::Duration::seconds(crate::storage::WAKE_ATTEMPT_LEASE_SECS))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            match storage.take_wake_attempt(watch.id, pending, &cutoff).await {
                Ok(true) => {}
                Ok(false) => return FireOutcome::Deferred("another attempt holds the wake"),
                Err(_) => return FireOutcome::Deferred("lease failed"),
            }
        }
        // The look that DECIDES: a wake that cannot happen now claims nothing,
        // and the halt it finds is the only halt this decision may clear.
        let decided_halt = match self.admission(bridge, storage, &watch, pending, false).await {
            Admission::Admit { halt, .. } => halt,
            Admission::Defer(why) => return FireOutcome::Deferred(why),
            Admission::Drop(why) => {
                self.drop_watch(storage, &watch, &why).await;
                return FireOutcome::Dropped(why);
            }
        };
        if !resuming {
            // The rate limit, for a NEW claim only: a resumed claim finishes.
            if let Some(at) = watch.woke_at.as_deref().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()) {
                let since = chrono::Utc::now().signed_duration_since(at.with_timezone(&chrono::Utc));
                if since < chrono::Duration::from_std(Self::min_gap(bridge)).unwrap_or_default() {
                    return FireOutcome::Deferred("rate limited");
                }
            }
            match storage.claim_control_watch_wake(watch.id, pending).await {
                Ok(true) => {}
                Ok(false) => return FireOutcome::AlreadyFired,
                Err(_) => return FireOutcome::Deferred("claim failed"),
            }
        }
        self.deliver_wake(bridge, storage, &watch, pending, decided_halt).await
    }

    /// Notice once (idempotent by its `watch:<watch>:<event>` link), then the
    /// release-boundary recheck, then the release. An undelivered claim stays
    /// a claim for the next retry; a claim the boundary refuses for a changed
    /// world is handed back so the same pending event can fire later.
    async fn deliver_wake(&self, bridge: &SignalingBridge, storage: &Storage, watch: &ControlWatch, pending: i64, decided_halt: DecidedHalt) -> FireOutcome {
        let notice_id = match storage.wake_notice_message_id(watch, pending).await {
            Ok(Some(id)) => id,
            Ok(None) => match self.write_notice(bridge, storage, watch, pending).await {
                Some(id) => id,
                None => {
                    let _ = storage.release_wake_attempt(watch.id, pending).await;
                    return FireOutcome::Deferred("notice not written");
                }
            },
            Err(_) => {
                let _ = storage.release_wake_attempt(watch.id, pending).await;
                return FireOutcome::Deferred("notice lookup failed");
            }
        };
        self.barrier_step(BarrierPoint::AfterNotice).await;
        let (actor, halt, attempt) = match self.admission(bridge, storage, watch, pending, true).await {
            // A halt declared after the decision — the controller's own, the
            // host's, anyone's — is not the decision's to clear: withhold, and
            // a later decision may look again.
            Admission::Admit { halt, attempt, .. } if halt != decided_halt => {
                let _ = storage.unclaim_control_watch_wake(watch.id, pending, attempt.as_deref()).await;
                tracing::info!(watch = watch.id, "watch wake withheld: the halt changed after the decision");
                return FireOutcome::Deferred("newer halt");
            }
            Admission::Admit { actor, halt, attempt } => (actor, halt, attempt),
            Admission::Defer(why) => {
                // The world changed after the notice: hand the claim back; the
                // notice stands (it was true when written) and is not repeated.
                let _ = storage.unclaim_control_watch_wake(watch.id, pending, None).await;
                tracing::info!(watch = watch.id, why, "watch wake withheld at the release boundary");
                return FireOutcome::Deferred(why);
            }
            Admission::Drop(why) => {
                self.drop_watch(storage, watch, &why).await;
                return FireOutcome::Dropped(why);
            }
        };
        let audit = storage.start_control_audit(&actor, Some(&watch.session_id), "watch_wake").await.ok();
        self.barrier_step(BarrierPoint::BeforeClear).await;
        // The halt: by id, or by its whole observed legacy slot, so a newer
        // halt is never cleared by this decision — and when the clear finds a
        // newer one, the release does not happen.
        let clear = match &halt {
            DecidedHalt::Id(id) => Some(storage.clear_session_halt_if_id(&watch.session_id, id).await),
            DecidedHalt::Legacy { by, reason, at } => Some(storage.clear_session_halt_if_legacy(&watch.session_id, by, reason, at).await),
            DecidedHalt::None => None,
        };
        let cleared = match clear {
            None => false,
            Some(Ok(true)) => true,
            Some(Ok(false)) => {
                let _ = storage.unclaim_control_watch_wake(watch.id, pending, attempt.as_deref()).await;
                if let Some(audit) = audit {
                    let _ = storage.finish_control_audit(audit, Some("withheld: a newer halt stands")).await;
                }
                return FireOutcome::Deferred("newer halt");
            }
            Some(Err(_)) => {
                let _ = storage.unclaim_control_watch_wake(watch.id, pending, attempt.as_deref()).await;
                return FireOutcome::Deferred("halt clear failed");
            }
        };
        if cleared {
            bridge.notify_halts_cleared(watch.session_id.clone());
        }
        let Some(ring) = bridge.session_ring(&watch.session_id).await else {
            let _ = storage.unclaim_control_watch_wake(watch.id, pending, attempt.as_deref()).await;
            return FireOutcome::Deferred("no ring");
        };
        bridge.clear_session_awaiting(&watch.session_id).await;
        // Never a `UserMessage`: a `HostWake` is dropped under Pause, admitted
        // only against the ring's own idle/halt/gate state AND the watch's
        // current claim and grant, deduped by the ring's own delivery receipt,
        // and acknowledged back (`watch_wake_consumed` / `watch_wake_withheld`)
        // under the attempt's lease stamp — the ring, which owns the deal, is
        // the last authority and the one that turns the claim into a delivery.
        if ring
            .try_send(crate::core::sequencer::SequencerCommand::HostWake {
                participant_id: watch.participant_id,
                watch_id: watch.id,
                pending,
                attempt: attempt.clone().unwrap_or_default(),
            })
            .is_err()
        {
            // The ring did not take it: the claim stays (the notice is
            // written), the attempt's lease is dropped, and the next retry
            // releases.
            tracing::warn!(watch = watch.id, "the ring did not take the wake; retrying later");
            let _ = storage.release_wake_attempt(watch.id, pending).await;
            if let Some(audit) = audit {
                let _ = storage.finish_control_audit(audit, Some("the ring did not take the wake")).await;
            }
            return FireOutcome::Deferred("ring did not take the wake");
        }
        tracing::info!(watch = watch.id, session = %watch.session_id, slug = %watch.slug, pending, notice_id, "watch wake handed to the ring");
        if let Some(audit) = audit {
            let _ = storage.finish_control_audit(audit, None).await;
        }
        FireOutcome::Released
    }

    /// The system row in the controller's session, linked as the wake's.
    async fn write_notice(&self, bridge: &SignalingBridge, storage: &Storage, watch: &ControlWatch, pending: i64) -> Option<i64> {
        let controllers = self.controller_sessions(storage).await;
        let pending_row = storage.control_event_by_id(pending).await.ok().flatten();
        let mut waiting = 0usize;
        let mut from = watch.cursor;
        loop {
            let Ok(page) = storage.control_events_after(from, REARM_PAGE).await else { break };
            waiting += page.iter().filter(|e| watch_matches(watch, e, &controllers)).count();
            match page.last() {
                Some(last) if page.len() == REARM_PAGE => from = last.id,
                _ => break,
            }
        }
        let waiting = waiting.max(1);
        let what = match pending_row {
            Some(e) => match (&e.agent, &e.detail) {
                (Some(agent), Some(detail)) => format!("{} in {} by {agent}: {detail}", e.kind, e.session_id),
                (Some(agent), None) => format!("{} in {} by {agent}", e.kind, e.session_id),
                (None, Some(detail)) => format!("{} in {}: {detail}", e.kind, e.session_id),
                (None, None) => format!("{} in {}", e.kind, e.session_id),
            },
            None => format!("event #{pending} (no longer retained; its kind was {})", watch.pending_kind.as_deref().unwrap_or("unknown")),
        };
        let halt_note = match storage.session_halt(&watch.session_id).await.ok().flatten() {
            Some((by, reason, _)) => format!(" This wake releases your halt (\"{reason}\", declared by {by}) unless a newer stop has landed."),
            None => String::new(),
        };
        let text = format!(
            "[System: watch {} woke {} — {waiting} unacknowledged event{} since #{}, first #{pending}: {what}.{halt_note} \
             Read them with app_control events {{since_id: {}}} and acknowledge with ack {{request_id: \"{}\", up_to: <next_since_id>}}; \
             no further wake for this watch until then.]",
            watch.request_id,
            watch.slug,
            if waiting == 1 { "" } else { "s" },
            watch.cursor,
            watch.cursor,
            watch.request_id
        );
        // Notice and link in one transaction: a second attempt's notice rolls
        // back when the link already exists, so no orphan is ever posted.
        let id = match storage.post_wake_notice(&watch.session_id, watch.participant_id, &format!("watch:{}:{pending}", watch.id), &text).await {
            Ok(Some(id)) => id,
            Ok(None) => match storage.wake_notice_message_id(watch, pending).await {
                Ok(Some(id)) => id,
                _ => return None,
            },
            Err(e) => {
                tracing::warn!(?e, watch = watch.id, "watch wake notice could not be written");
                return None;
            }
        };
        bridge.notify_message_persisted(Arc::from(watch.session_id.as_str()), id);
        Some(id)
    }
}

impl SignalingBridge {
    /// `app_control watch`: create or update a watch of the caller's. Refuses
    /// the caller's own session as a target, unknown kinds, and more than
    /// [`MAX_WATCHES_PER_CONTROLLER`] watches. A new watch starts at the
    /// journal's end; an update keeps the cursor and re-arms against its new
    /// filters.
    pub(crate) async fn control_watch_upsert(
        &self,
        storage: &Storage,
        actor: &ControlActor,
        request_id: &str,
        targets: Option<Vec<String>>,
        kinds: Option<Vec<String>>,
        wake: bool,
    ) -> Result<ControlWatch> {
        anyhow::ensure!(!request_id.trim().is_empty() && request_id.len() <= 120, "request_id must be 1–120 characters");
        if let Some(targets) = &targets {
            anyhow::ensure!(!targets.is_empty(), "sessions must name at least one session, or be omitted for every other session");
            anyhow::ensure!(
                !targets.iter().any(|t| *t == actor.session_id),
                "a watch never covers the controller's own session ({}): its own rows reach it as a participant",
                actor.session_id
            );
            for target in targets {
                anyhow::ensure!(storage.get_session(target).await?.is_some(), "session {target} does not exist");
            }
        }
        if let Some(kinds) = &kinds {
            anyhow::ensure!(!kinds.is_empty(), "kinds must name at least one kind, or be omitted for the attention kinds");
            for kind in kinds {
                anyhow::ensure!(WATCHABLE_KINDS.contains(&kind.as_str()), "unknown event kind `{kind}`; known: {}", WATCHABLE_KINDS.join(", "));
            }
        }
        let existing = storage.control_watch(actor.participant_id, request_id).await?;
        if existing.is_none() {
            let count = storage.control_watches_for(actor.participant_id).await?.len();
            anyhow::ensure!(count < MAX_WATCHES_PER_CONTROLLER, "a controller holds at most {MAX_WATCHES_PER_CONTROLLER} watches; unwatch one first");
        }
        let end = storage.max_control_event_id().await?;
        let watch = storage
            .upsert_control_watch(&actor.session_id, actor.participant_id, &actor.slug, request_id, targets.as_deref(), kinds.as_deref(), wake, end)
            .await?;
        if existing.is_some() {
            // Filters changed: the pending marker is re-derived from the cursor.
            sqlx::query("UPDATE control_watches SET pending_since = NULL WHERE id = ?").bind(watch.id).execute(storage.pool()).await?;
        }
        let engine = WatchEngine::new();
        engine.rearm_from_cursor(self, storage, watch.id).await?.context("the watch vanished")
    }

    /// `app_control ack`: the explicit, per-watch, monotonic acknowledgement.
    pub(crate) async fn control_watch_ack(&self, storage: &Storage, actor: &ControlActor, request_id: &str, up_to: i64) -> Result<ControlWatch> {
        let watch = storage.control_watch(actor.participant_id, request_id).await?.context("no such watch")?;
        anyhow::ensure!(up_to >= 0, "up_to must be a journal id");
        let end = storage.max_control_event_id().await?;
        storage.ack_control_watch(watch.id, up_to, end).await?.context("the watch vanished")?;
        WatchEngine::new().rearm_from_cursor(self, storage, watch.id).await?.context("the watch vanished")
    }

    pub(crate) async fn control_watch_remove(&self, storage: &Storage, actor: &ControlActor, request_id: &str) -> Result<bool> {
        storage.delete_control_watch(actor.participant_id, request_id).await
    }

    /// A ring came up for `session_id`: its pending watches may fire now.
    pub async fn rearm_watches_for(&self, session_id: &str) {
        let Some(storage) = self.storage_handle().await else { return };
        WatchEngine::new().retry_pending_for_session(self, &storage, session_id).await;
    }

    /// The scheduler tick: every wake watch, re-armed or retried — what fires
    /// a wake deferred for busyness, a gate or the rate limit once nothing
    /// else prompts it.
    pub async fn retry_all_watches(&self) {
        let Some(storage) = self.storage_handle().await else { return };
        WatchEngine::new().rearm_all(self, &storage).await;
    }

    /// The ring reports a `HostWake` delivered: the claim becomes a delivery
    /// only on the ring's own durable receipt — the notice in
    /// `participant_deliveries` for the controller. No receipt, or none
    /// readable, records nothing (eyes-2 `4fe7a60d`).
    pub async fn watch_wake_consumed(&self, session_id: &str, watch_id: i64, pending: i64) {
        let Some(storage) = self.storage_handle().await else { return };
        let Ok(Some(watch)) = storage.control_watch_by_id(watch_id).await else { return };
        let delivered = match storage.wake_notice_message_id(&watch, pending).await {
            Ok(Some(notice)) => storage.wake_notice_delivered(watch.participant_id, notice).await,
            Ok(None) => Ok(false),
            Err(e) => Err(e),
        };
        match delivered {
            Ok(true) => match storage.mark_control_watch_delivered(watch_id, pending).await {
                Ok(true) => tracing::info!(session_id, watch_id, pending, "watch wake delivered"),
                Ok(false) => tracing::debug!(session_id, watch_id, pending, "watch wake consumed for a claim that no longer stands"),
                Err(e) => tracing::warn!(?e, watch_id, "could not record the watch wake's delivery"),
            },
            Ok(false) => tracing::debug!(session_id, watch_id, pending, "watch wake reported consumed but its notice has no delivery receipt; the claim stays"),
            Err(e) => tracing::warn!(?e, watch_id, "could not read the watch wake's receipt; the claim stays"),
        }
    }

    /// The ring withheld a `HostWake` (paused, a turn live, a gate open, a
    /// halt standing): the claim goes back so a later look may fire again —
    /// only the claim this attempt holds (`attempt` is its lease stamp), so a
    /// stale outcome cannot undo a successor's.
    pub async fn watch_wake_withheld(&self, session_id: &str, watch_id: i64, pending: i64, attempt: &str, why: &str) {
        let Some(storage) = self.storage_handle().await else { return };
        tracing::info!(session_id, watch_id, pending, why, "watch wake withheld by the ring");
        let fence = (!attempt.is_empty()).then_some(attempt);
        if let Err(e) = storage.unclaim_control_watch_wake(watch_id, pending, fence).await {
            tracing::warn!(?e, watch_id, "could not hand the watch wake's claim back");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::sequencer::SequencerCommand;
    use crate::storage::QuestionKind;

    fn watch(session: &str, targets: Option<&[&str]>, kinds: Option<&[&str]>) -> ControlWatch {
        ControlWatch {
            id: 1,
            session_id: session.into(),
            participant_id: 1,
            slug: "hands".into(),
            request_id: "w".into(),
            targets: targets.map(|t| t.iter().map(|s| s.to_string()).collect()),
            kinds: kinds.map(|k| k.iter().map(|s| s.to_string()).collect()),
            wake: true,
            cursor: 0,
            pending_since: None,
            pending_kind: None,
            woke_for: None,
            woke_at: None,
            wake_attempt_at: None,
            wake_delivered_at: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn ev(session: &str, kind: &str) -> ControlEvent {
        ControlEvent {
            id: 1,
            at: String::new(),
            session_id: session.into(),
            kind: kind.into(),
            agent: None,
            ref_id: None,
            detail: None,
            source_session: None,
            source_slug: None,
        }
    }

    /// The matching rules, one by one: own session never; `watch_wake` never;
    /// own-caused never; default targets skip controller sessions; default
    /// kinds are the attention kinds; explicit lists are exact.
    #[test]
    fn a_watch_matches_only_what_its_filters_and_the_loop_guards_allow() {
        let controllers = vec!["b".to_string()];
        let default = watch("a", None, None);
        assert!(watch_matches(&default, &ev("t", "decision_pending"), &controllers));
        assert!(watch_matches(&default, &ev("t", "halt_declared"), &controllers));
        assert!(watch_matches(&default, &ev("t", "gate_urgent"), &controllers));
        // Feedback #144: by default, health wakes only for a runtime that
        // DIED. The stall watchdog flips running↔stalled at 90 s without a
        // tool event — a slow reasoning model crosses that routinely, and
        // each flip woke the controller (four paid turns in nine minutes).
        let health = |detail: Option<&str>| {
            let mut e = ev("t", "health");
            e.detail = detail.map(str::to_string);
            e
        };
        assert!(watch_matches(&default, &health(Some("dead")), &controllers), "a dead runtime wakes by default");
        assert!(!watch_matches(&default, &health(Some("stalled")), &controllers), "a stall is transient");
        assert!(!watch_matches(&default, &health(Some("running")), &controllers));
        assert!(!watch_matches(&default, &health(Some("retrying")), &controllers), "the supervisor is handling it");
        assert!(!watch_matches(&default, &health(None), &controllers), "no detail: nothing known to have died");
        let every_transition = watch("a", None, Some(&["health"]));
        assert!(watch_matches(&every_transition, &health(Some("stalled")), &controllers), "naming health keeps every transition");
        assert!(watch_matches(&every_transition, &health(Some("running")), &controllers));
        assert!(watch_matches(&default, &ev("t", "session_closed"), &controllers));
        assert!(!watch_matches(&default, &ev("t", "message"), &controllers), "chatter does not wake by default");
        assert!(!watch_matches(&default, &ev("t", "activity"), &controllers));
        assert!(!watch_matches(&default, &ev("a", "decision_pending"), &controllers), "never its own session");
        assert!(!watch_matches(&default, &ev("b", "decision_pending"), &controllers), "another controller's session is not 'every other session'");
        assert!(!watch_matches(&default, &ev("*", "journal_gap"), &controllers), "markers are not attention kinds");
        let named = watch("a", Some(&["b", "t", "*"]), Some(&["message", "journal_gap"]));
        assert!(watch_matches(&named, &ev("b", "message"), &controllers), "named explicitly, a controller's session counts");
        assert!(watch_matches(&named, &ev("*", "journal_gap"), &controllers), "the journal's own markers, when named as a target");
        assert!(!watch_matches(&watch("a", Some(&["t"]), Some(&["journal_gap"])), &ev("*", "journal_gap"), &controllers), "not when the targets leave `*` out");
        assert!(!watch_matches(&named, &ev("t", "decision_pending"), &controllers), "named kinds are exact");
        assert!(!watch_matches(&named, &ev("u", "message"), &controllers));
        assert!(!watch_matches(&named, &ev("a", "message"), &controllers), "own session never, even named");
        let mut wake = ev("t", "watch_wake");
        assert!(!watch_matches(&watch("a", Some(&["t"]), Some(&["watch_wake"])), &wake, &controllers), "a wake's notice never arms anything");
        wake.kind = "message".into();
        wake.source_session = Some("a".into());
        wake.source_slug = Some("hands".into());
        assert!(!watch_matches(&watch("a", Some(&["t"]), Some(&["message"])), &wake, &controllers), "a row the controller caused never arms its watch");
        wake.source_slug = Some("eyes".into());
        assert!(watch_matches(&watch("a", Some(&["t"]), Some(&["message"])), &wake, &controllers), "another participant's row does");
    }

    struct Rig {
        bridge: Arc<SignalingBridge>,
        storage: Storage,
        journal: Arc<super::super::control_events::ControlEventJournal>,
        rx: tokio::sync::mpsc::Receiver<SequencerCommand>,
        tracker: Arc<crate::core::activity::ActivityTracker>,
        hands: i64,
    }

    /// Controller session `c` (hands granted, ring registered, tracker idle)
    /// and a target session `t`.
    async fn rig() -> Rig {
        let bridge = SignalingBridge::new();
        let storage = Storage::memory().await.unwrap();
        bridge.set_storage(storage.clone()).await;
        for (id, title) in [("c", "controller"), ("t", "target")] {
            storage.create_session(id, title, None).await.unwrap();
            storage.ensure_session_roster(id, 2).await.unwrap();
        }
        sqlx::query("UPDATE roles SET capabilities = '[\"control_bot_hq\"]' WHERE slug = 'hands'").execute(storage.pool()).await.unwrap();
        sqlx::query("UPDATE session_participants SET capabilities = '[\"control_bot_hq\"]' WHERE session_id = 'c' AND slug = 'hands'").execute(storage.pool()).await.unwrap();
        let hands = storage.participant_by_slug("c", "hands").await.unwrap().unwrap().id;
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        bridge.register_session_sequencer("c".into(), tx).await;
        let awaiting = Arc::new(AtomicBool::new(false));
        bridge.register_session_awaiting("c".into(), Arc::clone(&awaiting)).await;
        let tracker = crate::core::activity::ActivityTracker::new("c", awaiting, bridge.clone(), vec!["hands".into(), "eyes".into()]);
        bridge.register_session_activity("c".into(), Arc::downgrade(&tracker)).await;
        let journal = bridge.control_event_journal_for_tests();
        // The rate limit is a backstop; these tests exercise the mechanism.
        bridge.watch_min_gap_secs.store(0, Ordering::Relaxed);
        Rig { bridge, storage, journal, rx, tracker, hands }
    }

    fn drain(rx: &mut tokio::sync::mpsc::Receiver<SequencerCommand>) -> Vec<SequencerCommand> {
        let mut out = Vec::new();
        while let Ok(c) = rx.try_recv() {
            out.push(c);
        }
        out
    }

    /// What the real ring does with a `HostWake` it admits: deal the notice
    /// (its delivery receipt) and tell the bridge, which turns the claim into
    /// a delivery on that receipt. Returns the wakes `(participant, watch,
    /// pending)` the channel held.
    async fn pump(bridge: &SignalingBridge, session: &str, rx: &mut tokio::sync::mpsc::Receiver<SequencerCommand>) -> Vec<(i64, i64, i64)> {
        let storage = bridge.storage_handle().await.unwrap();
        let mut wakes = Vec::new();
        for cmd in drain(rx) {
            match cmd {
                SequencerCommand::HostWake { participant_id, watch_id, pending, .. } => {
                    let watch = storage.control_watch_by_id(watch_id).await.unwrap().unwrap();
                    if let Some(notice) = storage.wake_notice_message_id(&watch, pending).await.unwrap() {
                        storage.commit_delivery(participant_id, &[(notice, None)]).await.unwrap();
                    }
                    bridge.watch_wake_consumed(session, watch_id, pending).await;
                    wakes.push((participant_id, watch_id, pending));
                }
                SequencerCommand::UserMessage { .. } => panic!("a watch wake is never a UserMessage (a fresh one releases a Pause)"),
                _ => {}
            }
        }
        wakes
    }

    async fn park_decision(storage: &Storage, session: &str, choice: &str) {
        let yes_no = ["yes".to_string(), "no".to_string()];
        storage.insert_tray_entry(session, choice, "hands", QuestionKind::Choice, "Deploy?", Some(&yes_no), None, None).await.unwrap();
    }

    fn decision(session: &str, choice: &str) -> SignalingEvent {
        SignalingEvent::PendingChoice(PendingChoice {
            choice_id: choice.into(),
            session_id: session.into(),
            agent: "hands".into(),
            question: "Deploy?".into(),
            options: vec!["yes".into(), "no".into()],
            approval: None,
        })
    }

    /// #132/#135: omitted targets means every matching non-controller
    /// session, including one created after this waking watch was registered.
    #[tokio::test]
    async fn a_default_waking_watch_covers_a_session_created_after_the_watch() {
        let r = rig().await;
        let actor = r.storage.controller_actor("c", "hands").await.unwrap();
        let initial = r.bridge.control_watch_upsert(&r.storage, &actor, "late", None, None, true).await.unwrap();
        assert!(initial.targets.is_none() && initial.kinds.is_none());
        assert!(initial.wake && initial.pending_since.is_none());

        r.storage.create_session("late-target", "later work", None).await.unwrap();
        r.storage.ensure_session_roster("late-target", 2).await.unwrap();
        // rig grants the global HANDS role control. The later roster inherits
        // it, so make this a non-controller target without revoking our actor.
        sqlx::query("UPDATE session_participants SET capabilities = '[]' WHERE session_id = 'late-target'")
            .execute(r.storage.pool()).await.unwrap();
        assert!(!r.storage.controller_sessions_open().await.unwrap().iter().any(|s| s == "late-target"));

        park_decision(&r.storage, "late-target", "late-choice").await;
        r.journal.record(&decision("late-target", "late-choice")).await;

        let armed = r.storage.control_watch(r.hands, "late").await.unwrap().unwrap();
        let pending = armed.pending_since.expect("a later target must arm the watch");
        let event = r.storage.control_event_by_id(pending).await.unwrap().unwrap();
        assert_eq!((armed.id, armed.cursor), (initial.id, initial.cursor));
        assert!(armed.targets.is_none());
        assert_eq!(armed.pending_kind.as_deref(), Some("decision_pending"));
        assert!(pending > initial.cursor);
        assert_eq!(
            (event.session_id.as_str(), event.kind.as_str(), event.ref_id.as_deref()),
            ("late-target", "decision_pending", Some("late-choice")),
        );
    }

    /// Feedback #144, the upgrade residue: a default watch left PENDING on a
    /// `health: stalled` row by a build that matched every health transition
    /// must not wake under the dead-only rule. The retry re-validates the
    /// persisted pending event, disarms it WITHOUT moving the ack cursor, and
    /// the watch re-arms from that cursor — so a later `dead` wakes, and a
    /// swept pending health row (kind kept on the watch, detail unknowable) is
    /// treated as stale too. Explicit `kinds:["health"]` keeps the old wake.
    #[tokio::test]
    async fn a_pending_transient_health_event_is_disarmed_not_woken() {
        let mut r = rig().await;
        let actor = r.storage.controller_actor("c", "hands").await.unwrap();
        let w = r.bridge.control_watch_upsert(&r.storage, &actor, "w1", None, None, true).await.unwrap();
        let engine = r.journal.watches();
        drain(&mut r.rx);
        let stalled = r.storage.insert_control_event(&NewControlEvent { session_id: "t".into(), kind: "health".into(), agent: Some("eyes".into()), detail: Some("stalled".into()), ..Default::default() }).await.unwrap();
        // Armed the way the old build left it.
        assert!(r.storage.arm_control_watch(w.id, stalled, "health").await.unwrap());
        assert_eq!(engine.try_fire(&r.bridge, &r.storage, w.id).await, FireOutcome::NothingPending, "a transient health state is not a wake");
        let after = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        assert_eq!((after.pending_since, after.pending_kind.as_deref(), after.woke_for), (None, None, None), "disarmed: {after:?}");
        assert_eq!(after.cursor, w.cursor, "the ack cursor is the controller's alone");
        assert!(pump(&r.bridge, "c", &mut r.rx).await.is_empty(), "nothing handed to the ring");
        // A dead runtime after it: found from the cursor and woken.
        let dead = r.storage.insert_control_event(&NewControlEvent { session_id: "t".into(), kind: "health".into(), agent: Some("eyes".into()), detail: Some("dead".into()), ..Default::default() }).await.unwrap();
        r.bridge.retry_all_watches().await;
        let woke = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        assert_eq!((woke.pending_since, woke.woke_for), (Some(dead), Some(dead)), "{woke:?}");
        assert_eq!(pump(&r.bridge, "c", &mut r.rx).await.len(), 1);
        // A swept pending health row under default kinds: stale, disarmed.
        let end = r.storage.max_control_event_id().await.unwrap();
        r.bridge.control_watch_ack(&r.storage, &actor, "w1", end).await.unwrap();
        let swept = r.storage.insert_control_event(&NewControlEvent { session_id: "t".into(), kind: "health".into(), detail: Some("stalled".into()), ..Default::default() }).await.unwrap();
        assert!(r.storage.arm_control_watch(w.id, swept, "health").await.unwrap());
        sqlx::query("DELETE FROM control_events WHERE id = ?").bind(swept).execute(r.storage.pool()).await.unwrap();
        assert_eq!(engine.try_fire(&r.bridge, &r.storage, w.id).await, FireOutcome::NothingPending);
        assert!(r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap().pending_since.is_none());
        // Named health keeps every transition, persisted or live.
        let named = r.bridge.control_watch_upsert(&r.storage, &actor, "w2", None, Some(vec!["health".to_string()]), true).await.unwrap();
        let stalled2 = r.storage.insert_control_event(&NewControlEvent { session_id: "t".into(), kind: "health".into(), agent: Some("eyes".into()), detail: Some("stalled".into()), ..Default::default() }).await.unwrap();
        assert!(r.storage.arm_control_watch(named.id, stalled2, "health").await.unwrap());
        assert_eq!(engine.try_fire(&r.bridge, &r.storage, named.id).await, FireOutcome::Released);
    }

    /// The wake, end to end through the real bridge: a decision parked in the
    /// target arms the watch and fires once — a system row naming the watch
    /// and the event, linked as `watch:` so the journal files it `watch_wake`;
    /// the controller's halt cleared by id; the ring released with the
    /// controller mentioned; audit row; no second wake for a second event
    /// until `ack`, which re-arms from the first unacknowledged match.
    #[tokio::test]
    async fn a_matching_event_wakes_the_halted_controller_once_per_acknowledgement() {
        let mut r = rig().await;
        let actor = r.storage.controller_actor("c", "hands").await.unwrap();
        let w = r.bridge.control_watch_upsert(&r.storage, &actor, "w1", None, None, true).await.unwrap();
        assert_eq!((w.pending_since, w.wake), (None, true));
        let halt_id = r.storage.declare_session_halt_stamped("c", "hands", "waiting for events").await.unwrap().unwrap();
        let mut events = r.bridge.subscribe();
        drain(&mut r.rx);

        park_decision(&r.storage, "t", "q1").await;
        r.journal.record(&decision("t", "q1")).await;

        let w = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        let pending = w.pending_since.expect("armed");
        assert_eq!((w.woke_for, w.wake_delivered_at.is_some()), (Some(pending), false), "claimed and handed to the ring, not yet consumed: {w:?}");
        let rows = r.storage.messages_for_session("c", None).await.unwrap();
        let notice = rows.iter().find(|m| m.kind == "system_notice" && m.content.contains("watch w1 woke hands")).expect("the wake notice");
        assert!(notice.content.contains("decision_pending in t by hands: Deploy?"), "{}", notice.content);
        assert!(notice.content.contains("This wake releases your halt (\"waiting for events\", declared by hands) unless a newer stop has landed."), "{}", notice.content);
        assert!(notice.content.contains(&format!("app_control events {{since_id: {}}}", w.cursor)), "{}", notice.content);
        assert!(notice.content.contains("ack {request_id: \"w1\""), "{}", notice.content);
        assert_eq!(r.storage.delegated_message_source(notice.id).await.unwrap(), Some(("c".into(), "hands".into(), format!("watch:{}:{pending}", w.id))));
        assert!(r.storage.session_halt("c").await.unwrap().is_none(), "the controller's halt is released");
        assert!(r.storage.session_halt_id("c").await.unwrap().is_none());
        let _ = halt_id;
        let wakes = pump(&r.bridge, "c", &mut r.rx).await;
        assert_eq!(wakes, vec![(r.hands, w.id, pending)], "a HostWake for the controller, never a UserMessage");
        let w = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        assert!(w.wake_delivered_at.is_some(), "the ring's acknowledgement is the delivery: {w:?}");
        let mut cleared = false;
        while let Ok(ev) = events.try_recv() {
            cleared |= matches!(ev, SignalingEvent::HaltsCleared { .. });
        }
        assert!(cleared);
        let audit = r.storage.control_audit_page(Some("c"), None).await.unwrap();
        assert!(audit.iter().any(|a| a.operation == "watch_wake" && a.outcome == "ok"), "{audit:?}");
        // The notice itself, journaled, is `watch_wake` and arms nothing.
        r.journal.record(&SignalingEvent::MessagePersisted { session_id: Arc::from("c"), message_id: notice.id }).await;
        let journaled = r.storage.control_events_page(0, None, None, 100).await.unwrap().events;
        assert!(journaled.iter().any(|e| e.kind == "watch_wake" && e.session_id == "c" && e.source_slug.as_deref() == Some("hands")), "{journaled:#?}");
        // A second matching event: pending stays the first, no second wake.
        park_decision(&r.storage, "t", "q2").await;
        r.journal.record(&decision("t", "q2")).await;
        let w2 = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        assert_eq!((w2.pending_since, w2.woke_for), (Some(pending), Some(pending)));
        assert!(pump(&r.bridge, "c", &mut r.rx).await.is_empty(), "no second wake before an ack");
        // Ack up to the first event: the second re-arms and, the controller
        // being idle and unhalted, fires again.
        let w3 = r.bridge.control_watch_ack(&r.storage, &actor, "w1", pending).await.unwrap();
        assert_eq!(w3.cursor, pending);
        let second = w3.pending_since.expect("re-armed from the first remaining match");
        assert!(second > pending);
        assert_eq!(w3.woke_for, Some(second), "woken for the newer event");
        assert_eq!(pump(&r.bridge, "c", &mut r.rx).await.len(), 1);
        // Ack everything: idle, nothing pending; a backwards ack is a no-op.
        let end = r.storage.max_control_event_id().await.unwrap();
        let w4 = r.bridge.control_watch_ack(&r.storage, &actor, "w1", end).await.unwrap();
        assert_eq!((w4.cursor, w4.pending_since), (end, None));
        let w5 = r.bridge.control_watch_ack(&r.storage, &actor, "w1", 0).await.unwrap();
        assert_eq!(w5.cursor, end);
        assert_eq!(r.storage.control_watches_for(r.hands).await.unwrap().len(), 1);
        assert!(r.bridge.control_watch_remove(&r.storage, &actor, "w1").await.unwrap());
        assert!(r.storage.control_watches_for(r.hands).await.unwrap().is_empty());
    }

    /// Every refusal at the release boundary keeps the event pending and
    /// fires nothing: a human Pause, a busy controller, a pending approval
    /// gate, no ring (then the ring's registration fires it); a confirmed loss
    /// of the grant drops the watch and journals it; a `message` event does
    /// not arm a default watch, nor does a row the controller caused.
    #[tokio::test]
    async fn a_wake_is_refused_at_the_boundary_and_retried_or_dropped() {
        let mut r = rig().await;
        let actor = r.storage.controller_actor("c", "hands").await.unwrap();
        r.bridge.control_watch_upsert(&r.storage, &actor, "w1", None, None, true).await.unwrap();
        r.storage.declare_session_halt("c", "hands", "waiting").await.unwrap();
        drain(&mut r.rx);
        let engine = r.journal.watches();
        let watch_id = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap().id;
        // Paused: armed, not fired.
        r.tracker.set_paused(true);
        park_decision(&r.storage, "t", "q1").await;
        r.journal.record(&decision("t", "q1")).await;
        let w = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        assert!(w.pending_since.is_some() && w.woke_for.is_none(), "{w:?}");
        assert!(pump(&r.bridge, "c", &mut r.rx).await.is_empty());
        assert_eq!(engine.try_fire(&r.bridge, &r.storage, watch_id).await, FireOutcome::Deferred("paused"));
        assert!(r.storage.session_halt("c").await.unwrap().is_some(), "a wake never releases a human Pause");
        r.tracker.set_paused(false);
        // Busy: still deferred.
        r.tracker.set_busy_slug("hands", true);
        assert_eq!(engine.try_fire(&r.bridge, &r.storage, watch_id).await, FireOutcome::Deferred("busy"));
        r.tracker.set_busy_slug("hands", false);
        // A pending approval gate in the controller's session: deferred.
        r.storage.insert_tray_entry("c", "g1", "hands", QuestionKind::Approval, "git push", Some(&["Approve".to_string(), "Reject".to_string()]), None, Some("git push")).await.unwrap();
        assert_eq!(engine.try_fire(&r.bridge, &r.storage, watch_id).await, FireOutcome::Deferred("gate pending"));
        sqlx::query("UPDATE session_tray SET status = 'answered' WHERE choice_id = 'g1'").execute(r.storage.pool()).await.unwrap();
        // The gate resolving is a change in the controller's own session: the
        // journal row for it retries the pending watch, which now fires.
        r.journal.record(&SignalingEvent::ChoiceResolved { choice_id: "g1".into(), picked: "Approve".into() }).await;
        let w = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        assert_eq!(w.woke_for, w.pending_since, "fired once the gate was answered: {w:?}");
        assert_eq!(pump(&r.bridge, "c", &mut r.rx).await.len(), 1);
        assert_eq!(engine.try_fire(&r.bridge, &r.storage, watch_id).await, FireOutcome::AlreadyFired);
        // No ring: a fresh controller session with a watch and no sequencer.
        r.storage.create_session("d", "dormant controller", None).await.unwrap();
        r.storage.ensure_session_roster("d", 2).await.unwrap();
        sqlx::query("UPDATE session_participants SET capabilities = '[\"control_bot_hq\"]' WHERE session_id = 'd' AND slug = 'hands'").execute(r.storage.pool()).await.unwrap();
        let d_actor = r.storage.controller_actor("d", "hands").await.unwrap();
        r.bridge.control_watch_upsert(&r.storage, &d_actor, "wd", Some(vec!["t".to_string()]), None, true).await.unwrap();
        park_decision(&r.storage, "t", "q2").await;
        r.journal.record(&decision("t", "q2")).await;
        let wd = r.storage.control_watch(d_actor.participant_id, "wd").await.unwrap().unwrap();
        assert!(wd.pending_since.is_some() && wd.woke_for.is_none(), "armed, no ring: {wd:?}");
        assert_eq!(engine.try_fire(&r.bridge, &r.storage, wd.id).await, FireOutcome::Deferred("no ring"));
        assert_eq!(r.storage.sessions_with_pending_attention_watch().await.unwrap(), ["d"], "a boot would respawn it");
        let (tx_d, mut rx_d) = tokio::sync::mpsc::channel(16);
        r.bridge.register_session_sequencer("d".into(), tx_d).await;
        let wd = r.storage.control_watch(d_actor.participant_id, "wd").await.unwrap().unwrap();
        assert_eq!(wd.woke_for, wd.pending_since, "the ring's registration fired it: {wd:?}");
        assert_eq!(pump(&r.bridge, "d", &mut rx_d).await.len(), 1);
        // Revocation: the role loses the grant; the next fire drops the watch.
        let end = r.storage.max_control_event_id().await.unwrap();
        r.bridge.control_watch_ack(&r.storage, &actor, "w1", end).await.unwrap();
        sqlx::query("UPDATE roles SET capabilities = '[]' WHERE slug = 'hands'").execute(r.storage.pool()).await.unwrap();
        park_decision(&r.storage, "t", "q3").await;
        r.journal.record(&decision("t", "q3")).await;
        assert!(r.storage.control_watch(r.hands, "w1").await.unwrap().is_none(), "dropped on a confirmed loss");
        let dropped = r.storage.control_events_page(0, None, Some(&["watch_dropped".to_string()]), 100).await.unwrap().events;
        assert_eq!(dropped.len(), 1, "c's watch, at its own trigger: {dropped:#?}");
        assert!(dropped[0].detail.as_deref().unwrap().contains("revoked") && dropped[0].source_slug.as_deref() == Some("hands") && dropped[0].session_id == "c");
        assert!(pump(&r.bridge, "c", &mut r.rx).await.is_empty());
        // d's watch, already fired for its pending event, is untouched until
        // something retries it — then it is dropped the same way.
        assert!(r.storage.control_watch(d_actor.participant_id, "wd").await.unwrap().is_some());
        r.bridge.rearm_watches_for("d").await;
        assert!(r.storage.control_watch(d_actor.participant_id, "wd").await.unwrap().is_none());
        assert_eq!(r.storage.control_events_page(0, None, Some(&["watch_dropped".to_string()]), 100).await.unwrap().events.len(), 2);
        assert!(pump(&r.bridge, "d", &mut rx_d).await.is_empty());
    }

    /// Two controllers, both watching "everything" with wakes, one external
    /// event: each wakes exactly once, and everything the two wakes produce —
    /// the notices, the cleared halts, the activity flips, their replies —
    /// arms neither, so there is no chain (EYES, Plan 5c review).
    #[tokio::test]
    async fn two_controllers_watching_everything_do_not_wake_each_other() {
        let mut r = rig().await;
        // A second controller session `b`, same shape as `c`.
        r.storage.create_session("b", "controller b", None).await.unwrap();
        r.storage.ensure_session_roster("b", 2).await.unwrap();
        sqlx::query("UPDATE session_participants SET capabilities = '[\"control_bot_hq\"]' WHERE session_id = 'b' AND slug = 'hands'").execute(r.storage.pool()).await.unwrap();
        let (tx_b, mut rx_b) = tokio::sync::mpsc::channel(16);
        r.bridge.register_session_sequencer("b".into(), tx_b).await;
        let awaiting_b = Arc::new(AtomicBool::new(false));
        let tracker_b = crate::core::activity::ActivityTracker::new("b", awaiting_b, r.bridge.clone(), vec!["hands".into()]);
        r.bridge.register_session_activity("b".into(), Arc::downgrade(&tracker_b)).await;
        let c_actor = r.storage.controller_actor("c", "hands").await.unwrap();
        let b_actor = r.storage.controller_actor("b", "hands").await.unwrap();
        r.bridge.control_watch_upsert(&r.storage, &c_actor, "all", None, None, true).await.unwrap();
        r.bridge.control_watch_upsert(&r.storage, &b_actor, "all", None, None, true).await.unwrap();
        r.storage.declare_session_halt("c", "hands", "waiting").await.unwrap();
        r.storage.declare_session_halt("b", "hands", "waiting").await.unwrap();
        drain(&mut r.rx);
        drain(&mut rx_b);

        park_decision(&r.storage, "t", "q1").await;
        r.journal.record(&decision("t", "q1")).await;
        assert_eq!(pump(&r.bridge, "c", &mut r.rx).await.len(), 1, "c woke once");
        assert_eq!(pump(&r.bridge, "b", &mut rx_b).await.len(), 1, "b woke once");
        // Everything the wakes produced, as the journal task would see it.
        for session in ["c", "b"] {
            for m in r.storage.messages_for_session(session, None).await.unwrap() {
                r.journal.record(&SignalingEvent::MessagePersisted { session_id: Arc::from(session), message_id: m.id }).await;
            }
            r.journal.record(&SignalingEvent::HaltsCleared { session_id: session.into() }).await;
            r.journal.record(&SignalingEvent::SessionActivity { session_id: session.into(), state: "busy".into(), slot0_busy: true, slot1_busy: false, busy_slots: vec![0] }).await;
            let reply = r.storage.post_to_channel(session, "participant", Some("hands"), "text", "reading the events", None).await.unwrap().message_id();
            r.journal.record(&SignalingEvent::MessagePersisted { session_id: Arc::from(session), message_id: reply }).await;
            r.journal.record(&SignalingEvent::AwaitingUser { session_id: session.into(), agent: "hands".into(), reason: "waiting again".into(), halt_id: None }).await;
        }
        assert!(pump(&r.bridge, "c", &mut r.rx).await.is_empty(), "nothing a wake produced woke c again");
        assert!(pump(&r.bridge, "b", &mut rx_b).await.is_empty(), "nothing a wake produced woke b again");
        for (actor, _) in [(&c_actor, "c"), (&b_actor, "b")] {
            let w = r.storage.control_watch(actor.participant_id, "all").await.unwrap().unwrap();
            assert_eq!(w.woke_for, w.pending_since, "still the first event, already fired: {w:?}");
        }
        // The other controller's halt is attention-kind but lives in a
        // controller session: excluded from "every other session" — even
        // after an ack, nothing re-arms from it.
        let end = r.storage.max_control_event_id().await.unwrap();
        let w = r.bridge.control_watch_ack(&r.storage, &c_actor, "all", end).await.unwrap();
        assert_eq!(w.pending_since, None, "{w:?}");
        assert!(pump(&r.bridge, "c", &mut r.rx).await.is_empty());
    }

    /// eyes-2 `22085139`: the world changes AFTER the notice is written and
    /// BEFORE the release — a human Pause, a newer halt, a revocation, an
    /// unwatch, a new live turn. Each is seen at the release boundary: nothing
    /// is released, the claim is handed back (or the watch dropped), and the
    /// only trace is the notice already written.
    #[tokio::test]
    async fn the_release_boundary_sees_what_changed_after_the_notice() {
        let mut r = rig().await;
        let actor = r.storage.controller_actor("c", "hands").await.unwrap();
        r.bridge.control_watch_upsert(&r.storage, &actor, "w1", None, None, true).await.unwrap();
        let watch_id = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap().id;
        let engine = r.journal.watches();
        let storage = r.storage.clone();
        let tracker = r.tracker.clone();

        // 1. A newer halt lands between the notice and the clear.
        r.storage.declare_session_halt("c", "hands", "first").await.unwrap();
        let storage_b = storage.clone();
        *engine.barrier.lock().await = Some((BarrierPoint::AfterNotice, Box::new(move || {
            let s = storage_b.clone();
            Box::pin(async move { s.declare_session_halt("c", "hands", "newer").await.unwrap(); })
        })));
        park_decision(&r.storage, "t", "q1").await;
        drain(&mut r.rx);
        r.journal.record(&decision("t", "q1")).await;
        let w = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        let pending = w.pending_since.expect("armed");
        assert_eq!(w.woke_for, None, "the claim was handed back: {w:?}");
        assert!(pump(&r.bridge, "c", &mut r.rx).await.is_empty(), "nothing released");
        let (_, reason, _) = r.storage.session_halt("c").await.unwrap().expect("the newer halt stands");
        assert_eq!(reason, "newer");
        let notices = r.storage.messages_for_session("c", None).await.unwrap().into_iter().filter(|m| m.content.contains("watch w1 woke")).count();
        assert_eq!(notices, 1, "the notice written before the change stands, once");
        // Nothing changes under the next retry: it fires without a second notice.
        *engine.barrier.lock().await = None;
        assert_eq!(engine.try_fire(&r.bridge, &r.storage, watch_id).await, FireOutcome::Released);
        assert_eq!(pump(&r.bridge, "c", &mut r.rx).await.len(), 1);
        assert!(r.storage.session_halt("c").await.unwrap().is_none(), "the halt that stood AT the release is the one cleared");
        let notices = r.storage.messages_for_session("c", None).await.unwrap().into_iter().filter(|m| m.content.contains("watch w1 woke")).count();
        assert_eq!(notices, 1, "a resumed claim never repeats its notice");
        let w = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        assert_eq!((w.woke_for, w.wake_delivered_at.is_some()), (Some(pending), true));

        // 2. A human Pause lands after the notice.
        let end = r.storage.max_control_event_id().await.unwrap();
        r.bridge.control_watch_ack(&r.storage, &actor, "w1", end).await.unwrap();
        let tracker_b = tracker.clone();
        *engine.barrier.lock().await = Some((BarrierPoint::AfterNotice, Box::new(move || {
            tracker_b.set_paused(true);
            Box::pin(async {})
        })));
        park_decision(&r.storage, "t", "q2").await;
        r.journal.record(&decision("t", "q2")).await;
        let w = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        assert!(w.pending_since.is_some() && w.woke_for.is_none(), "paused at the boundary: handed back: {w:?}");
        assert!(pump(&r.bridge, "c", &mut r.rx).await.is_empty(), "a wake never releases a Pause");
        *engine.barrier.lock().await = None;
        assert_eq!(engine.try_fire(&r.bridge, &r.storage, watch_id).await, FireOutcome::Deferred("paused"));
        tracker.set_paused(false);
        assert_eq!(engine.try_fire(&r.bridge, &r.storage, watch_id).await, FireOutcome::Released);
        assert_eq!(pump(&r.bridge, "c", &mut r.rx).await.len(), 1);

        // 3. A live turn starts after the notice.
        let end = r.storage.max_control_event_id().await.unwrap();
        r.bridge.control_watch_ack(&r.storage, &actor, "w1", end).await.unwrap();
        let tracker_b = tracker.clone();
        *engine.barrier.lock().await = Some((BarrierPoint::AfterNotice, Box::new(move || {
            tracker_b.set_busy_slug("hands", true);
            Box::pin(async {})
        })));
        park_decision(&r.storage, "t", "q3").await;
        r.journal.record(&decision("t", "q3")).await;
        assert!(pump(&r.bridge, "c", &mut r.rx).await.is_empty(), "a working controller is not interrupted");
        assert_eq!(r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap().woke_for, None);
        *engine.barrier.lock().await = None;
        tracker.set_busy_slug("hands", false);

        // 4. The watch is removed after the notice.
        let storage_b = storage.clone();
        *engine.barrier.lock().await = Some((BarrierPoint::AfterNotice, Box::new(move || {
            let s = storage_b.clone();
            Box::pin(async move { s.delete_control_watch_by_id(watch_id).await.unwrap(); })
        })));
        assert_eq!(engine.try_fire(&r.bridge, &r.storage, watch_id).await, FireOutcome::Dropped("the watch was removed".into()));
        assert!(pump(&r.bridge, "c", &mut r.rx).await.is_empty());
        *engine.barrier.lock().await = None;

        // 5. The grant is revoked after the notice.
        let actor = r.storage.controller_actor("c", "hands").await.unwrap();
        r.bridge.control_watch_upsert(&r.storage, &actor, "w2", None, None, true).await.unwrap();
        let w2 = r.storage.control_watch(r.hands, "w2").await.unwrap().unwrap();
        let storage_b = storage.clone();
        *engine.barrier.lock().await = Some((BarrierPoint::AfterNotice, Box::new(move || {
            let s = storage_b.clone();
            Box::pin(async move {
                sqlx::query("UPDATE roles SET capabilities = '[]' WHERE slug = 'hands'").execute(s.pool()).await.unwrap();
            })
        })));
        park_decision(&r.storage, "t", "q4").await;
        r.journal.record(&decision("t", "q4")).await;
        assert!(r.storage.control_watch_by_id(w2.id).await.unwrap().is_none(), "dropped at the boundary");
        assert!(pump(&r.bridge, "c", &mut r.rx).await.is_empty());
        *engine.barrier.lock().await = None;
    }

    /// eyes-2 `08b6e015`: a claim is intent, not delivery. A crash after the
    /// claim (woke_for set, nothing delivered) is resumed: the notice is
    /// written once and the ring released once; a crash after the notice but
    /// before the ring resumes without a second notice; the boot query lists
    /// the controller until delivery; a ring that will not take the wake
    /// keeps the claim for the next retry.
    #[tokio::test]
    async fn an_undelivered_claim_is_resumed_not_repeated() {
        let mut r = rig().await;
        let actor = r.storage.controller_actor("c", "hands").await.unwrap();
        r.bridge.control_watch_upsert(&r.storage, &actor, "w1", None, None, true).await.unwrap();
        let w = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        let engine = r.journal.watches();
        // "Crash after the claim": arm and claim by hand, deliver nothing.
        let e1 = r.storage.insert_control_event(&NewControlEvent { session_id: "t".into(), kind: "decision_pending".into(), agent: Some("hands".into()), ref_id: Some("q1".into()), detail: Some("Deploy?".into()), ..Default::default() }).await.unwrap();
        assert!(r.storage.arm_control_watch(w.id, e1, "decision_pending").await.unwrap());
        assert!(r.storage.claim_control_watch_wake(w.id, e1).await.unwrap());
        assert_eq!(r.storage.sessions_with_pending_attention_watch().await.unwrap(), ["c"], "a boot would respawn the controller: the wake is owed");
        drain(&mut r.rx);
        // The crashed attempt's lease still stands: a second resumer waits.
        assert_eq!(engine.try_fire(&r.bridge, &r.storage, w.id).await, FireOutcome::Deferred("another attempt holds the wake"));
        sqlx::query("UPDATE control_watches SET wake_attempt_at = '2000-01-01T00:00:00.000Z' WHERE id = ?").bind(w.id).execute(r.storage.pool()).await.unwrap();
        assert_eq!(engine.try_fire(&r.bridge, &r.storage, w.id).await, FireOutcome::Released, "resumed once the lease lapsed");
        let notices = r.storage.messages_for_session("c", None).await.unwrap().into_iter().filter(|m| m.content.contains("watch w1 woke")).count();
        assert_eq!(notices, 1);
        assert_eq!(pump(&r.bridge, "c", &mut r.rx).await.len(), 1);
        assert_eq!(engine.try_fire(&r.bridge, &r.storage, w.id).await, FireOutcome::AlreadyFired);
        assert!(r.storage.sessions_with_pending_attention_watch().await.unwrap().is_empty());
        // "Crash after the notice, before the ring": the link exists, the
        // claim stands undelivered — resumed without a second notice.
        let end = r.storage.max_control_event_id().await.unwrap();
        r.bridge.control_watch_ack(&r.storage, &actor, "w1", end).await.unwrap();
        let e2 = r.storage.insert_control_event(&NewControlEvent { session_id: "t".into(), kind: "halt_declared".into(), agent: Some("eyes".into()), detail: Some("CI".into()), ..Default::default() }).await.unwrap();
        assert!(r.storage.arm_control_watch(w.id, e2, "halt_declared").await.unwrap());
        assert!(r.storage.claim_control_watch_wake(w.id, e2).await.unwrap());
        let notice = r.storage.post_wake_notice("c", r.hands, &format!("watch:{}:{e2}", w.id), "[System: watch w1 woke hands — written before the crash]").await.unwrap().expect("written");
        assert_eq!(r.storage.post_wake_notice("c", r.hands, &format!("watch:{}:{e2}", w.id), "[System: watch w1 woke hands — a losing attempt]").await.unwrap(), None, "the link refuses a second notice and the notice rolls back with it");
        assert_eq!(r.storage.wake_notice_message_id(&w, e2).await.unwrap(), Some(notice));
        sqlx::query("UPDATE control_watches SET wake_attempt_at = '2000-01-01T00:00:00.000Z' WHERE id = ?").bind(w.id).execute(r.storage.pool()).await.unwrap();
        assert_eq!(engine.try_fire(&r.bridge, &r.storage, w.id).await, FireOutcome::Released);
        let notices = r.storage.messages_for_session("c", None).await.unwrap().into_iter().filter(|m| m.content.contains("watch w1 woke")).count();
        assert_eq!(notices, 2, "the crashed attempt's notice plus the first wake's; no third");
        assert_eq!(pump(&r.bridge, "c", &mut r.rx).await.len(), 1);
        // A ring that will not take the wake: the claim and notice stand, the
        // release is retried. (A full channel: 16 slots, filled.)
        let end = r.storage.max_control_event_id().await.unwrap();
        r.bridge.control_watch_ack(&r.storage, &actor, "w1", end).await.unwrap();
        let ring = r.bridge.session_ring("c").await.unwrap();
        while ring.try_send(SequencerCommand::Pause).is_ok() {}
        park_decision(&r.storage, "t", "q3").await;
        r.journal.record(&decision("t", "q3")).await;
        let w3 = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        assert!(w3.woke_for.is_some() && w3.wake_delivered_at.is_none(), "claimed, notice written, release owed: {w3:?}");
        drain(&mut r.rx);
        assert_eq!(engine.try_fire(&r.bridge, &r.storage, w.id).await, FireOutcome::Released);
        assert_eq!(pump(&r.bridge, "c", &mut r.rx).await.len(), 1);
        let notices = r.storage.messages_for_session("c", None).await.unwrap().into_iter().filter(|m| m.content.contains("watch w1 woke")).count();
        assert_eq!(notices, 3, "one notice for the third wake, not two");
    }

    /// eyes-2 `9c503a55` / `5b89f957` / gap: the re-arm pages to the
    /// journal's end past hundreds of non-matching rows; a row that committed
    /// without the engine seeing it (journaled-but-unarmed) is found by the
    /// start-up / tick re-arm; a persisted pending decision whose broadcast
    /// was dropped wakes a default watcher after `gap`; a pending event the
    /// retention swept is still described and still counts for boot.
    #[tokio::test]
    async fn rearm_pages_past_the_window_and_recovers_what_the_engine_never_saw() {
        let mut r = rig().await;
        let actor = r.storage.controller_actor("c", "hands").await.unwrap();
        r.bridge.control_watch_upsert(&r.storage, &actor, "w1", None, None, true).await.unwrap();
        let w = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        let engine = r.journal.watches();
        drain(&mut r.rx);
        // 600 non-matching rows (another participant's activity in t), then one match, all
        // written straight to the journal — the engine never saw them.
        for _ in 0..600 {
            r.storage.insert_control_event(&NewControlEvent { session_id: "t".into(), kind: "activity".into(), detail: Some("busy".into()), ..Default::default() }).await.unwrap();
        }
        let far = r.storage.insert_control_event(&NewControlEvent { session_id: "t".into(), kind: "decision_pending".into(), agent: Some("hands".into()), ref_id: Some("far".into()), detail: Some("Far?".into()), ..Default::default() }).await.unwrap();
        assert_eq!(r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap().pending_since, None, "unarmed: the engine never saw them");
        r.bridge.retry_all_watches().await;
        let w1 = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        assert_eq!((w1.pending_since, w1.woke_for), (Some(far), Some(far)), "found past 500 rows and fired: {w1:?}");
        assert_eq!(pump(&r.bridge, "c", &mut r.rx).await.len(), 1);
        // The same through `ack`: acknowledging re-arms by paging too.
        for _ in 0..600 {
            r.storage.insert_control_event(&NewControlEvent { session_id: "t".into(), kind: "activity".into(), detail: Some("idle".into()), ..Default::default() }).await.unwrap();
        }
        let farther = r.storage.insert_control_event(&NewControlEvent { session_id: "t".into(), kind: "gate_urgent".into(), agent: Some("hands".into()), ref_id: Some("g".into()), detail: Some("prod".into()), ..Default::default() }).await.unwrap();
        let w2 = r.bridge.control_watch_ack(&r.storage, &actor, "w1", far).await.unwrap();
        assert_eq!((w2.pending_since, w2.woke_for), (Some(farther), Some(farther)), "{w2:?}");
        assert_eq!(pump(&r.bridge, "c", &mut r.rx).await.len(), 1);
        // A dropped broadcast: the decision is durable, no PendingChoice ever
        // reached the journal; `gap` reconciles it and the watcher wakes.
        let end = r.storage.max_control_event_id().await.unwrap();
        r.bridge.control_watch_ack(&r.storage, &actor, "w1", end).await.unwrap();
        park_decision(&r.storage, "t", "lost").await;
        r.journal.gap(4).await;
        let w3 = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        let recovered = r.storage.control_event_by_id(w3.pending_since.unwrap()).await.unwrap().unwrap();
        assert_eq!((recovered.kind.as_str(), recovered.ref_id.as_deref()), ("decision_pending", Some("lost")));
        assert_eq!(w3.woke_for, w3.pending_since, "woke for the reconciled decision: {w3:?}");
        assert_eq!(pump(&r.bridge, "c", &mut r.rx).await.len(), 1);
        // The journal's start does the same for a journaled-but-unarmed row.
        let end = r.storage.max_control_event_id().await.unwrap();
        r.bridge.control_watch_ack(&r.storage, &actor, "w1", end).await.unwrap();
        let unseen = r.storage.insert_control_event(&NewControlEvent { session_id: "t".into(), kind: "health".into(), agent: Some("eyes".into()), detail: Some("dead".into()), ..Default::default() }).await.unwrap();
        r.journal.start().await;
        let w4 = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        assert_eq!((w4.pending_since, w4.woke_for), (Some(unseen), Some(unseen)), "{w4:?}");
        assert_eq!(pump(&r.bridge, "c", &mut r.rx).await.len(), 1);
        // A pending row the retention swept: the notice names the gap, the kind
        // kept on the watch still makes it a boot candidate.
        let end = r.storage.max_control_event_id().await.unwrap();
        r.bridge.control_watch_ack(&r.storage, &actor, "w1", end).await.unwrap();
        let swept = r.storage.insert_control_event(&NewControlEvent { session_id: "t".into(), kind: "session_closed".into(), ..Default::default() }).await.unwrap();
        assert!(r.storage.arm_control_watch(w.id, swept, "session_closed").await.unwrap());
        sqlx::query("DELETE FROM control_events WHERE id = ?").bind(swept).execute(r.storage.pool()).await.unwrap();
        assert_eq!(r.storage.sessions_with_pending_attention_watch().await.unwrap(), ["c"]);
        assert_eq!(engine.try_fire(&r.bridge, &r.storage, w.id).await, FireOutcome::Released);
        let last = r.storage.messages_for_session("c", None).await.unwrap().into_iter().filter(|m| m.content.contains("watch w1 woke")).last().unwrap();
        assert!(last.content.contains(&format!("event #{swept} (no longer retained; its kind was session_closed)")), "{}", last.content);
    }

    /// The rate limit: a second wake within the gap is deferred, the pending
    /// event stays, and the scheduler's retry fires it once the gap has passed.
    #[tokio::test]
    async fn a_watch_wakes_at_most_once_per_gap_and_the_tick_fires_the_rest() {
        let mut r = rig().await;
        r.bridge.watch_min_gap_secs.store(1, Ordering::Relaxed);
        let actor = r.storage.controller_actor("c", "hands").await.unwrap();
        r.bridge.control_watch_upsert(&r.storage, &actor, "w1", None, None, true).await.unwrap();
        let w = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        let engine = r.journal.watches();
        drain(&mut r.rx);
        park_decision(&r.storage, "t", "q1").await;
        r.journal.record(&decision("t", "q1")).await;
        assert_eq!(pump(&r.bridge, "c", &mut r.rx).await.len(), 1);
        let end = r.storage.max_control_event_id().await.unwrap();
        r.bridge.control_watch_ack(&r.storage, &actor, "w1", end).await.unwrap();
        park_decision(&r.storage, "t", "q2").await;
        r.journal.record(&decision("t", "q2")).await;
        let w2 = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        assert!(w2.pending_since.is_some() && w2.woke_for != w2.pending_since, "armed, rate limited: {w2:?}");
        assert_eq!(engine.try_fire(&r.bridge, &r.storage, w.id).await, FireOutcome::Deferred("rate limited"));
        assert!(pump(&r.bridge, "c", &mut r.rx).await.is_empty());
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        r.bridge.retry_all_watches().await;
        let w3 = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        assert_eq!(w3.woke_for, w3.pending_since, "the tick fired it after the gap: {w3:?}");
        assert_eq!(pump(&r.bridge, "c", &mut r.rx).await.len(), 1);
    }

    /// eyes-2 `8c22ab52`: a halt declared by a build before `halt_id` existed
    /// has no id. A newer halt (which has one) landing AFTER the release
    /// boundary's look and BEFORE the clear must not be erased: the legacy
    /// clear compares the whole observed slot, misses, withholds, and the
    /// newer halt stands.
    #[tokio::test]
    async fn a_legacy_halt_clear_never_erases_a_newer_halt() {
        let mut r = rig().await;
        let actor = r.storage.controller_actor("c", "hands").await.unwrap();
        r.bridge.control_watch_upsert(&r.storage, &actor, "w1", None, None, true).await.unwrap();
        let watch_id = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap().id;
        let engine = r.journal.watches();
        // A pre-0112 halt: declared by, reason and stamp, no id.
        sqlx::query("UPDATE sessions SET halt_declared_by = 'hands', halt_reason = 'old build halt', halt_declared_at = '2026-10-01T00:00:00.000Z', halt_id = NULL WHERE id = 'c'")
            .execute(r.storage.pool()).await.unwrap();
        assert_eq!(r.storage.session_halt_id("c").await.unwrap(), None);
        let storage_b = r.storage.clone();
        *engine.barrier.lock().await = Some((BarrierPoint::BeforeClear, Box::new(move || {
            let s = storage_b.clone();
            Box::pin(async move { s.declare_session_halt("c", "hands", "newer stop").await.unwrap(); })
        })));
        drain(&mut r.rx);
        park_decision(&r.storage, "t", "q1").await;
        r.journal.record(&decision("t", "q1")).await;
        let (_, reason, _) = r.storage.session_halt("c").await.unwrap().expect("the newer halt stands");
        assert_eq!(reason, "newer stop");
        assert!(r.storage.session_halt_id("c").await.unwrap().is_some());
        let w = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        assert!(w.pending_since.is_some() && w.woke_for.is_none(), "withheld, claim handed back: {w:?}");
        assert!(pump(&r.bridge, "c", &mut r.rx).await.is_empty(), "nothing was handed to the ring");
        // With nothing landing in the window, a legacy halt IS cleared by the
        // decision that observed it, and the wake goes to the ring.
        *engine.barrier.lock().await = None;
        sqlx::query("UPDATE sessions SET halt_declared_by = 'hands', halt_reason = 'old build halt', halt_declared_at = '2026-10-01T00:00:00.000Z', halt_id = NULL WHERE id = 'c'")
            .execute(r.storage.pool()).await.unwrap();
        assert_eq!(engine.try_fire(&r.bridge, &r.storage, watch_id).await, FireOutcome::Released);
        assert!(r.storage.session_halt("c").await.unwrap().is_none());
        assert_eq!(pump(&r.bridge, "c", &mut r.rx).await.len(), 1);
    }

    /// eyes-2 `4fe7a60d`: delivery is the notice's receipt and nothing else.
    /// A "consumed" report without a receipt records no delivery (a drain that
    /// stopped before the notice reached the controller keeps the wake
    /// recoverable); an unreadable receipt is unknown — the resume defers and
    /// never authorizes another release.
    #[tokio::test]
    async fn delivery_is_the_receipt_and_an_unreadable_receipt_decides_nothing() {
        let mut r = rig().await;
        let actor = r.storage.controller_actor("c", "hands").await.unwrap();
        r.bridge.control_watch_upsert(&r.storage, &actor, "w1", None, None, true).await.unwrap();
        let w = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        let engine = r.journal.watches();
        park_decision(&r.storage, "t", "q1").await;
        r.journal.record(&decision("t", "q1")).await;
        let w1 = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        let pending = w1.pending_since.unwrap();
        assert_eq!(w1.woke_for, Some(pending));
        // The ring reports consumption, but nothing was dealt (no receipt):
        // the claim stays undelivered.
        let cmds = drain(&mut r.rx);
        assert!(cmds.iter().any(|c| matches!(c, SequencerCommand::HostWake { .. })));
        r.bridge.watch_wake_consumed("c", w.id, pending).await;
        let w2 = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        assert!(w2.wake_delivered_at.is_none(), "no receipt, no delivery: {w2:?}");
        assert_eq!(r.storage.sessions_with_pending_attention_watch().await.unwrap(), ["c"], "still owed: a boot would respawn");
        // With the receipt (the deal landed the notice), the same report delivers.
        let notice = r.storage.wake_notice_message_id(&w, pending).await.unwrap().unwrap();
        r.storage.commit_delivery(r.hands, &[(notice, None)]).await.unwrap();
        r.bridge.watch_wake_consumed("c", w.id, pending).await;
        assert!(r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap().wake_delivered_at.is_some());
        // A resume whose receipt cannot be read defers — no second HostWake —
        // even with the marker wiped and the lease lapsed.
        sqlx::query("UPDATE control_watches SET wake_delivered_at = NULL, wake_attempt_at = '2000-01-01T00:00:00.000Z' WHERE id = ?").bind(w.id).execute(r.storage.pool()).await.unwrap();
        sqlx::query("DROP TABLE participant_deliveries").execute(r.storage.pool()).await.unwrap();
        assert_eq!(engine.try_fire(&r.bridge, &r.storage, w.id).await, FireOutcome::Deferred("receipt read failed"));
        assert!(drain(&mut r.rx).iter().all(|c| !matches!(c, SequencerCommand::HostWake { .. })), "nothing released on an unknown receipt");
        let w3 = r.storage.control_watch(r.hands, "w1").await.unwrap().unwrap();
        assert_eq!((w3.woke_for, w3.wake_delivered_at.clone()), (Some(pending), None), "the claim stays, recoverable: {w3:?}");
    }
}
