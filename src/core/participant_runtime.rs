//! Host-owned live participant changes. The durable request is intent; this
//! owner reserves the ring, manages processes and publishes the observed state.
use crate::agents::spawn::StopControl;
use crate::agents::{AgentHandle, RetryPolicy, SpawnConfig};
use crate::core::roster_boundary::RosterBoundary;
use crate::core::session::{refresh_runtime_roster, CandidateDraft, LiveCandidate, LiveContext};
use crate::core::state::AppState;
use crate::storage::{
    AgentConfig, ParticipantChange, ParticipantChangeOrigin, ParticipantChangeRequest,
};
use anyhow::{Context, Result};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;

type SpawnFn =
    dyn Fn(SpawnConfig) -> futures::future::BoxFuture<'static, Result<AgentHandle>> + Send + Sync;
#[derive(Clone)]
pub(crate) struct SpawnServices {
    pub spawn: Arc<SpawnFn>,
    pub auth: Arc<crate::core::session::AuthProbeFn>,
    pub codex: Arc<crate::core::session::CodexProbeFn>,
}

#[cfg(test)]
#[path = "participant_runtime_tests.rs"]
mod tests;
impl SpawnServices {
    fn native() -> Self {
        Self {
            spawn: Arc::new(|config| {
                Box::pin(crate::agents::spawn_supervised_agent(
                    config,
                    RetryPolicy::default(),
                ))
            }),
            auth: Arc::from(crate::core::session::live_auth_probe()),
            codex: Arc::from(crate::core::session::live_codex_probe()),
        }
    }
}

pub(crate) struct ChangeRun {
    pub id: String,
    pub operation: String,
    cancel: watch::Sender<bool>,
    done: watch::Receiver<bool>,
    controls: Mutex<Vec<StopControl>>,
}

pub(crate) struct ChangeCompletion(watch::Sender<bool>);
impl Drop for ChangeCompletion {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}

impl ChangeRun {
    pub fn new(operation: String) -> (Arc<Self>, ChangeCompletion) {
        let (cancel, _) = watch::channel(false);
        let (done_tx, done) = watch::channel(false);
        (
            Arc::new(Self {
                id: uuid::Uuid::new_v4().to_string(),
                operation,
                cancel,
                done,
                controls: Mutex::new(Vec::new()),
            }),
            ChangeCompletion(done_tx),
        )
    }
    pub fn cancel(&self) {
        self.cancel.send_replace(true);
    }
    pub fn is_cancelled(&self) -> bool {
        *self.cancel.borrow()
    }
    pub fn is_done(&self) -> bool {
        *self.done.borrow()
    }
    pub async fn cancelled(&self) {
        let mut signal = self.cancel.subscribe();
        while !*signal.borrow() {
            if signal.changed().await.is_err() {
                break;
            }
        }
    }
    pub fn track(&self, control: StopControl) {
        self.controls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(control);
    }
    pub async fn wait_done(&self) -> Result<()> {
        let mut done = self.done.clone();
        while !*done.borrow() {
            done.changed().await?;
        }
        Ok(())
    }
    pub async fn join_and_stop(&self) -> anyhow::Result<()> {
        self.wait_done().await?;
        let controls = self
            .controls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        for control in controls {
            control
                .retire_and_wait(std::time::Duration::from_secs(30))
                .await
                .map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
}

struct PreviousRuntime {
    participant: crate::storage::Participant,
    config: AgentConfig,
    control: StopControl,
    token: Option<String>,
}

impl AppState {
    /// Run outside the global session map lock. A retiring worker can need
    /// that lock to observe cancellation and clean up an unpublished child.
    pub(crate) async fn quiesce_session_handle(
        &self,
        handle: &mut crate::core::session::SessionHandle,
    ) -> Result<()> {
        if let Some(run) = &handle.change_run {
            run.cancel();
        }
        let scope = {
            let mut roster = handle
                .watchdog_roster
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            roster.changing = true;
            roster.scope.clone()
        };
        scope.retire().await;
        for actor in &handle.participants {
            if let Some(token) = &actor.mcp_token {
                self.bridge.revoke_mcp_token(&handle.id, &actor.slug, token);
            }
        }
        self.bridge.stop_session_ring(&handle.id).await;
        let mut first_error = None;
        // Issue all stops before waiting on one slow connector.
        for actor in &mut handle.participants {
            actor.handle.kill();
        }
        for actor in &mut handle.participants {
            if let Err(error) = actor.handle.retire_and_wait(Duration::from_secs(30)).await {
                first_error.get_or_insert(anyhow::anyhow!(error));
            }
        }
        if let Some(run) = &handle.change_run {
            if let Err(error) = run.join_and_stop().await {
                first_error.get_or_insert(error);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Host/UI entry. Provenance is fixed here; an agent facade must authorize
    /// its caller separately before a future delegated entry is exposed.
    pub async fn change_participant(
        self: &Arc<Self>,
        session: &str,
        id: &str,
        revision: i64,
        request: ParticipantChangeRequest,
    ) -> Result<ParticipantChange> {
        self.ensure_session_started(session).await?;
        self.start_participant_change(session, id, revision, request, SpawnServices::native())
            .await
    }

    pub async fn cancel_participant_change(&self, session: &str, id: &str) -> Result<bool> {
        let cancelled = self.storage.cancel_participant_change(session, id).await?;
        self.signal_participant_change_cancelled(session, id, cancelled).await;
        Ok(cancelled)
    }

    pub(crate) async fn cancel_participant_change_controlled(&self, session: &str, id: &str, claim: &crate::signaling::ControlClaim) -> Result<bool> {
        let cancelled = self.bridge.cancel_controlled_participant_change(claim, session, id).await?;
        self.signal_participant_change_cancelled(session, id, cancelled).await;
        if cancelled {
            crate::core::post_system_notice(&self.storage, Some(self.bridge.as_ref()), session,
                crate::storage::MessageKind::SystemNotice,
                format!("Participant change {id} was cancelled by delegated controller {} of {} before applying.", claim.actor.slug, claim.actor.session_id), None).await;
        }
        Ok(cancelled)
    }

    async fn signal_participant_change_cancelled(&self, session: &str, id: &str, cancelled: bool) {
        if cancelled {
            let sessions = self.sessions.lock().await;
            if let Some(run) = sessions
                .get(session)
                .and_then(|h| h.change_run.as_ref())
                .filter(|r| r.operation == id)
            {
                run.cancel();
            }
        }
    }

    pub(crate) async fn start_participant_change(
        self: &Arc<Self>,
        session: &str,
        id: &str,
        revision: i64,
        request: ParticipantChangeRequest,
        services: SpawnServices,
    ) -> Result<ParticipantChange> {
        self.start_participant_change_inner(session, id, revision, request, services, None).await
    }

    pub(crate) async fn change_participant_controlled(self: &Arc<Self>, session: &str, id: &str, revision: i64, request: ParticipantChangeRequest, claim: crate::signaling::ControlClaim) -> Result<ParticipantChange> {
        self.start_participant_change_inner(session, id, revision, request, SpawnServices::native(), Some(Arc::new(claim))).await
    }

    async fn start_participant_change_inner(self: &Arc<Self>, session: &str, id: &str, revision: i64, request: ParticipantChangeRequest, services: SpawnServices, control: Option<Arc<crate::signaling::ControlClaim>>) -> Result<ParticipantChange> {
        let _lifecycle = self.lifecycle_lock(session).lock_owned().await;
        let duplicate = {
            let sessions = self.sessions.lock().await;
            let handle = sessions
                .get(session)
                .context("session has no live runtime")?;
            if let Some(run) = handle.change_run.as_ref().filter(|r| !r.is_done()) {
                anyhow::ensure!(
                    run.operation == id,
                    "another participant change is still running"
                );
                true
            } else {
                false
            }
        };
        // A duplicate ID still validates the exact request; the DB call does
        // not occupy the global map while it waits for a writer.
        let row = match control.as_deref() {
            Some(claim) => self.bridge.request_controlled_participant_change(claim, session, id, revision, request).await?,
            None => self.storage.request_participant_change(session, id, revision, request, ParticipantChangeOrigin::User).await?,
        };
        if duplicate || row.status != "pending" {
            return Ok(row);
        }
        if let Some(claim) = control.as_deref() {
            crate::core::post_system_notice(&self.storage, Some(self.bridge.as_ref()), session,
                crate::storage::MessageKind::SystemNotice,
                format!("Participant change {} requested by delegated controller {} of {}. It will apply at a safe turn boundary.", row.id, claim.actor.slug, claim.actor.session_id), None).await;
        }
        let ring = self
            .bridge
            .session_ring(session)
            .await
            .context("session has no turn ring")?;
        let mut sessions = self.sessions.lock().await;
        let handle = sessions
            .get_mut(session)
            .context("session disappeared before its worker started")?;
        let context = LiveContext::capture(handle, ring);
        let (run, completion) = ChangeRun::new(id.to_string());
        handle.change_run = Some(run.clone());
        let this = self.clone();
        let row_for_worker = row.clone();
        tokio::spawn(async move {
            let _completion = completion;
            if let Err(error) = this
                .run_participant_change(&row_for_worker, &run, &context, &services, control.as_deref())
                .await
            {
                // Raw provider errors may include configuration/credentials.
                // Keep the recap useful without echoing a remote response.
                tracing::warn!(session_id = %row_for_worker.session_id, operation = %row_for_worker.id,
                    error = %error, "participant runtime change failed");
            }
        });
        Ok(row)
    }

    pub(super) fn owns_change(
        handle: &crate::core::session::SessionHandle,
        context: &LiveContext,
        run: &ChangeRun,
    ) -> bool {
        context.matches(handle)
            && !run.is_cancelled()
            && handle.change_run.as_ref().is_some_and(|r| r.id == run.id)
    }

    pub(super) async fn check_change_owner(
        &self,
        session: &str,
        run: &ChangeRun,
        context: &LiveContext,
    ) -> Result<()> {
        {
            let sessions = self.sessions.lock().await;
            anyhow::ensure!(
                sessions
                    .get(session)
                    .is_some_and(|h| Self::owns_change(h, context, run)),
                "participant change lost its parent runtime"
            );
        }
        let row = self
            .storage
            .get_session(session)
            .await?
            .context("session disappeared")?;
        anyhow::ensure!(
            row.closed_at.is_none() && row.archived == 0,
            "session closed during participant change"
        );
        Ok(())
    }

    /// Close/restart owns this same per-session barrier while joining the
    /// worker. Cancellation must therefore beat waiting for that barrier.
    pub(super) async fn publication_guard(
        &self,
        session: &str,
        run: &ChangeRun,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>> {
        let lifecycle = self.lifecycle_lock(session);
        tokio::select! {
            biased;
            _ = run.cancelled() => anyhow::bail!("participant change cancelled before publication"),
            guard = lifecycle.lock_owned() => Ok(guard),
        }
    }

    async fn previous_runtime(
        &self,
        row: &ParticipantChange,
        context: &LiveContext,
        run: &ChangeRun,
    ) -> Result<Option<PreviousRuntime>> {
        let id = match row.request()? {
            ParticipantChangeRequest::Add { .. } => return Ok(None),
            ParticipantChangeRequest::Remove { participant_id }
            | ParticipantChangeRequest::SwitchModel { participant_id, .. } => participant_id,
            ParticipantChangeRequest::SetEffort { .. } => anyhow::bail!("effort changes do not replace a runtime"),
        };
        let participant = self
            .storage
            .participants_for_session(&row.session_id)
            .await?
            .into_iter()
            .find(|p| p.id == id)
            .context("target is no longer in the roster")?;
        let sessions = self.sessions.lock().await;
        let handle = sessions
            .get(&row.session_id)
            .filter(|h| Self::owns_change(h, context, run))
            .context("parent runtime changed")?;
        let actor = handle
            .participants
            .iter()
            .find(|p| p.participant_id == Some(id))
            .context("target has no runtime")?;
        Ok(Some(PreviousRuntime {
            participant,
            config: actor
                .config_snapshot
                .clone()
                .context("target spawn configuration is unavailable")?,
            control: actor.handle.stop_control(),
            token: actor.mcp_token.clone(),
        }))
    }

    async fn run_participant_change(
        &self,
        row: &ParticipantChange,
        run: &ChangeRun,
        context: &LiveContext,
        services: &SpawnServices,
        control: Option<&crate::signaling::ControlClaim>,
    ) -> Result<()> {
        let boundary = tokio::select! {
            biased;
            result = RosterBoundary::acquire(context.ring.clone(), row.id.clone()) => match result {
                Ok(boundary) => boundary,
                Err(error) => { self.storage.cancel_participant_change(&row.session_id, &row.id).await?; return Err(anyhow::anyhow!(error)); }
            },
            _ = run.cancelled() => { self.storage.cancel_participant_change(&row.session_id, &row.id).await?; return Ok(()); }
        };
        if self
            .check_change_owner(&row.session_id, run, context)
            .await
            .is_err()
        {
            self.storage
                .cancel_participant_change(&row.session_id, &row.id)
                .await?;
            let same_parent = self
                .sessions
                .lock()
                .await
                .get(&row.session_id)
                .is_some_and(|h| context.matches(h));
            if same_parent {
                boundary.release().await.map_err(anyhow::Error::msg)?;
            } else {
                boundary.shutdown().await;
            }
            return Ok(());
        }
        let authority = self.bridge.participant_control_lock(&row.session_id).await;
        let _authority = authority.lock().await;
        let claimed = match control {
            Some(claim) => self.bridge.claim_controlled_participant_change(claim, &row.session_id, &row.id).await,
            None => self.storage.claim_participant_change(&row.session_id, &row.id).await,
        };
        let claimed = match claimed {
            Ok(row) => row,
            Err(error) => {
                self.storage.cancel_participant_change(&row.session_id, &row.id).await?;
                let error = crate::policy::secret_scan::redact_string(error.to_string());
                sqlx::query("UPDATE session_participant_changes SET error = ? WHERE id = ? AND status = 'cancelled'")
                    .bind(&error).bind(&row.id).execute(self.storage.pool()).await?;
                crate::core::post_system_notice(&self.storage, Some(self.bridge.as_ref()), &row.session_id,
                    crate::storage::MessageKind::SystemNotice,
                    format!("Participant change {} was cancelled before applying: {error}", row.id), None).await;
                boundary.release().await.map_err(anyhow::Error::msg)?;
                return Err(anyhow::anyhow!(error));
            }
        };
        if claimed.is_none() {
            boundary.release().await.map_err(anyhow::Error::msg)?;
            return Ok(());
        }
        if let ParticipantChangeRequest::SetEffort { participant_id, effort, ultracode } = row.request()? {
            return self.run_effort_change(row, run, context, boundary, control, participant_id,
                crate::agents::effort::EffortPair { effort, ultracode }).await;
        }
        context.hold_watchdog().await;
        let mut previous = None;
        let mut candidate = None;
        let mut candidate_token = None;
        let mut retired = false;
        let mut published = false;
        let mut notice = String::new();
        let result: Result<()> = async {
            previous = self.previous_runtime(row, context, run).await?;
            if let Some(old) = &previous {
                if let Some(token) = &old.token { self.bridge.revoke_mcp_token(&row.session_id, &old.participant.slug, token); }
                retired = true;
                old.control.retire_and_wait(Duration::from_secs(30)).await.map_err(anyhow::Error::msg)?;
                context.activity.set_busy_slug(&old.participant.slug, false);
            }
            self.check_change_owner(&row.session_id, run, context).await?;
            let prepared = self.storage.prepare_participant_change(&row.session_id, &row.id).await?;
            notice = if let Some(model) = &prepared.model {
                format!("{} now uses {} ({}). Capabilities: {}. This starts a fresh runtime with bounded session excerpts; saved history remains available.",
                    prepared.participant.slug, model.display_name, model.runtime, prepared.participant.capabilities)
            } else {
                format!("{} was removed from the active session. Its history, findings and pending user gates remain available.", prepared.participant.slug)
            };
            if let Some(model) = prepared.model {
                let config = crate::core::session::frozen_model_config(&prepared.participant.slug, &model);
                let draft = CandidateDraft::prepare(&self.storage, &self.paths, &self.bridge, self.signaling_addr,
                    context, prepared.participant, config, prepared.role, prepared.history_frontier, services).await?;
                // No participant writes or token registration happened during
                // the asynchronous probes. Recheck under the publication lock.
                {
                    let sessions = self.sessions.lock().await;
                    anyhow::ensure!(sessions.get(&row.session_id).is_some_and(|h| Self::owns_change(h, context, run)), "parent runtime changed during preparation");
                    self.bridge.register_mcp_token(&row.session_id, draft.slug(), draft.token());
                    candidate_token = Some((draft.slug().to_string(), draft.token().to_string()));
                }
                let fresh = draft.spawn(context, &self.bridge, services).await?;
                run.track(fresh.agent.handle.stop_control());
                candidate = Some(fresh);
            }
            self.check_change_owner(&row.session_id, run, context).await?;
            let target = candidate.as_ref().and_then(|c| c.agent.participant_id)
                .or_else(|| previous.as_ref().map(|p| p.participant.id)).context("no target participant")?;
            boundary.bind(target, candidate.as_ref().map(|c| (c.agent.handle.input().clone(), c.agent.turn_epoch.as_ref().unwrap().clone())))
                .await.map_err(anyhow::Error::msg)?;
            let _publication = self.publication_guard(&row.session_id, run).await?;
            self.check_change_owner(&row.session_id, run, context).await?;
            let roster = self.storage.participants_for_session(&row.session_id).await?;
            let committed = if let Some(fresh) = &candidate {
                self.storage.publish_participant_spawn(&row.session_id, &row.id, target, &fresh.metadata).await?
            } else { self.storage.finish_participant_change(&row.session_id, &row.id, None).await? };
            anyhow::ensure!(committed, "participant publication lost to session closure or recovery");
            published = true;
            // Reset before starting the new pump so its first real reading
            // cannot be erased by a delayed reset from this change.
            if let Some(changed) = roster.iter().find(|p| p.id == target) {
                self.bridge.notify_participant_changed(&row.session_id, &changed.slug);
            }
            {
                let mut sessions = self.sessions.lock().await;
                let handle = sessions.get_mut(&row.session_id).filter(|h| Self::owns_change(h, context, run)).context("parent runtime changed before installation")?;
                handle.participants.retain(|p| p.participant_id != Some(target));
                if let Some(fresh) = candidate.take() { self.install_candidate(handle, fresh, context); }
                refresh_runtime_roster(handle, &self.storage, &self.bridge, &roster);
            }
            self.storage.refresh_roster_handoffs(&row.session_id).await;
            self.bridge.reconcile_roster_publications(&row.session_id).await?;
            Ok(())
        }.await;
        if let Err(error) = result {
            if published {
                if let Some(fresh) = &mut candidate {
                    if let Some((slug, token)) = &candidate_token {
                        self.bridge.revoke_mcp_token(&row.session_id, slug, token);
                    }
                    fresh
                        .agent
                        .handle
                        .retire_and_wait(Duration::from_secs(30))
                        .await
                        .map_err(anyhow::Error::msg)?;
                }
                self.change_recap(&row.session_id, "The agent change was applied, but refreshing the session's pending work failed. Restart its agents to resume safely.").await;
                return Err(error);
            }
            if let Some(fresh) = &mut candidate {
                fresh
                    .agent
                    .handle
                    .retire_and_wait(Duration::from_secs(30))
                    .await
                    .map_err(anyhow::Error::msg)?;
            }
            if let Some((slug, token)) = &candidate_token {
                self.bridge.revoke_mcp_token(&row.session_id, slug, token);
            }
            let reason = "Participant change failed during runtime setup; restoring the previous configuration.";
            self.storage
                .finish_participant_change(&row.session_id, &row.id, Some(reason))
                .await?;
            if self
                .check_change_owner(&row.session_id, run, context)
                .await
                .is_err()
            {
                boundary.shutdown().await;
                return Err(error);
            }
            let recovery = self
                .restore_runtime(
                    row,
                    run,
                    context,
                    services,
                    &boundary,
                    previous.as_ref().filter(|_| retired),
                )
                .await;
            if recovery.is_err() {
                self.change_recap(&row.session_id, "The agent change failed and its runtime could not be restored. The session is stopped; restart its agents to recover.").await;
                // Drop the owner without release: no partial runtime may run.
                return recovery;
            }
            self.change_notice(&row.session_id, "The agent change failed. Its previous configuration and runtime are restored; the saved change records the failure.").await;
            boundary.release().await.map_err(anyhow::Error::msg)?;
            return Err(error);
        }
        self.change_notice(&row.session_id, &notice).await;
        boundary.release().await.map_err(anyhow::Error::msg)?;
        Ok(())
    }

    fn install_candidate(
        &self,
        handle: &mut crate::core::session::SessionHandle,
        fresh: LiveCandidate,
        context: &LiveContext,
    ) {
        let LiveCandidate {
            agent,
            events,
            pump,
            liveness,
            notices,
            ..
        } = fresh;
        handle.liveness.retain(|(slug, _)| slug != &agent.slug);
        handle
            .liveness
            .push((agent.slug.clone(), Arc::downgrade(&liveness)));
        self.bridge
            .notify_agent_health(handle.id.clone(), &agent.slug, "running");
        tokio::spawn(crate::core::pump::pump_agent(
            pump,
            events,
            self.storage.clone(),
            context.ipav.clone(),
        ));
        for notice in notices {
            let storage = self.storage.clone();
            let bridge = self.bridge.clone();
            let session = handle.id.clone();
            tokio::spawn(async move {
                crate::core::post_system_notice(
                    &storage,
                    Some(&bridge),
                    &session,
                    crate::storage::MessageKind::SystemNotice,
                    notice,
                    None,
                )
                .await;
            });
        }
        handle.participants.push(agent);
    }

    async fn restore_runtime(
        &self,
        row: &ParticipantChange,
        run: &ChangeRun,
        context: &LiveContext,
        services: &SpawnServices,
        boundary: &RosterBoundary,
        previous: Option<&PreviousRuntime>,
    ) -> Result<()> {
        let mut publication = None;
        if let Some(old) = previous {
            // A failed shutdown is not permission to start a second process.
            old.control
                .retire_and_wait(Duration::from_secs(30))
                .await
                .map_err(anyhow::Error::msg)?;
            let participant = self
                .storage
                .participant_by_slug(&row.session_id, &old.participant.slug)
                .await?
                .context("restored participant missing")?;
            anyhow::ensure!(
                participant.enabled
                    && participant.model_id == old.participant.model_id
                    && participant.runtime == old.participant.runtime,
                "restored participant changed again"
            );
            let revision = self.storage.roster_revision(&row.session_id).await?;
            let history_frontier = self.storage.cursor_for(old.participant.id).await?;
            let draft = CandidateDraft::prepare(
                &self.storage,
                &self.paths,
                &self.bridge,
                self.signaling_addr,
                context,
                participant,
                old.config.clone(),
                None,
                history_frontier,
                services,
            )
            .await?;
            self.check_change_owner(&row.session_id, run, context)
                .await?;
            let token = draft.token().to_string();
            self.bridge
                .register_mcp_token(&row.session_id, &old.participant.slug, &token);
            let fresh = draft.spawn(context, &self.bridge, services).await;
            let fresh = match fresh {
                Ok(fresh) => fresh,
                Err(error) => {
                    self.bridge
                        .revoke_mcp_token(&row.session_id, &old.participant.slug, &token);
                    return Err(error);
                }
            };
            run.track(fresh.agent.handle.stop_control());
            let mut fresh = Some(fresh);
            let restored: Result<()> = async {
                publication = Some(self.publication_guard(&row.session_id, run).await?);
                self.check_change_owner(&row.session_id, run, context)
                    .await?;
                let candidate = fresh.as_ref().unwrap();
                boundary
                    .bind(
                        old.participant.id,
                        Some((
                            candidate.agent.handle.input().clone(),
                            candidate.agent.turn_epoch.as_ref().unwrap().clone(),
                        )),
                    )
                    .await
                    .map_err(anyhow::Error::msg)?;
                anyhow::ensure!(
                    self.storage
                        .publish_recovered_participant_spawn(
                            &row.session_id,
                            &row.id,
                            old.participant.id,
                            revision,
                            history_frontier,
                            &candidate.metadata
                        )
                        .await?,
                    "runtime recovery lost publication ownership"
                );
                let mut sessions = self.sessions.lock().await;
                let handle = sessions
                    .get_mut(&row.session_id)
                    .filter(|h| Self::owns_change(h, context, run))
                    .context("parent runtime changed during recovery")?;
                handle
                    .participants
                    .retain(|p| p.participant_id != Some(old.participant.id));
                self.bridge.notify_participant_changed(&row.session_id, &old.participant.slug);
                self.install_candidate(handle, fresh.take().unwrap(), context);
                Ok(())
            }
            .await;
            if let Err(error) = restored {
                self.bridge
                    .revoke_mcp_token(&row.session_id, &old.participant.slug, &token);
                if let Some(fresh) = &mut fresh {
                    fresh
                        .agent
                        .handle
                        .retire_and_wait(Duration::from_secs(30))
                        .await
                        .map_err(anyhow::Error::msg)?;
                }
                return Err(error);
            }
        }
        if publication.is_none() {
            publication = Some(self.publication_guard(&row.session_id, run).await?);
        }
        self.check_change_owner(&row.session_id, run, context)
            .await?;
        let roster = self
            .storage
            .participants_for_session(&row.session_id)
            .await?;
        // A failed Add can have bound an input before publication refused.
        for participant in roster.iter().filter(|p| !p.enabled) {
            boundary
                .bind(participant.id, None)
                .await
                .map_err(anyhow::Error::msg)?;
        }
        {
            let mut sessions = self.sessions.lock().await;
            let handle = sessions
                .get_mut(&row.session_id)
                .filter(|h| Self::owns_change(h, context, run))
                .context("parent runtime changed after rollback")?;
            refresh_runtime_roster(handle, &self.storage, &self.bridge, &roster);
        }
        self.storage.refresh_roster_handoffs(&row.session_id).await;
        self.bridge
            .reconcile_roster_publications(&row.session_id)
            .await?;
        drop(publication);
        Ok(())
    }

    pub(super) async fn change_notice(&self, session: &str, text: &str) {
        crate::core::post_system_notice(
            &self.storage,
            Some(&self.bridge),
            session,
            crate::storage::MessageKind::SystemNotice,
            text.to_string(),
            None,
        )
        .await;
    }
    pub(super) async fn change_recap(&self, session: &str, text: &str) {
        self.change_notice(session, text).await;
        self.bridge
            .declare_host_halt(session, text.to_string())
            .await;
    }
}
