//! Confirmed knob updates at the existing roster boundary, without replacing
//! the participant. A failed/unconfirmed rollback quarantines its process.
use super::{
    participant_runtime::ChangeRun,
    roster_boundary::RosterBoundary,
    session::{refresh_runtime_roster, LiveContext},
    AppState,
};
use crate::{
    agents::effort::{EffortPair, EffortStatus},
    storage::{ParticipantChange, Storage},
};
use anyhow::Result;

impl AppState {
    pub(crate) async fn participant_effort_status(
        &self,
        session: &str,
    ) -> Vec<(i64, EffortStatus)> {
        let controls = {
            let sessions = self.sessions.lock().await;
            sessions
                .get(session)
                .map(|h| {
                    h.participants
                        .iter()
                        .filter_map(|p| {
                            p.participant_id
                                .map(|id| (id, p.handle.effort_control.clone()))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        futures::future::join_all(
            controls
                .into_iter()
                .map(|(id, control)| async move { (id, control.status().await) }),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_effort_change(
        &self,
        row: &ParticipantChange,
        run: &ChangeRun,
        context: &LiveContext,
        boundary: RosterBoundary,
        claim: Option<&crate::signaling::ControlClaim>,
        target: i64,
        desired: EffortPair,
    ) -> Result<()> {
        let pause = context.activity.pause_generation();
        let actor = {
            let sessions = self.sessions.lock().await;
            sessions
                .get(&row.session_id)
                .filter(|h| Self::owns_change(h, context, run))
                .and_then(|handle| {
                    handle
                        .participants
                        .iter()
                        .find(|p| p.participant_id == Some(target))
                })
                .map(|p| {
                    (
                        p.slug.clone(),
                        p.handle.effort_control.clone(),
                        p.handle.stop_control(),
                    )
                })
        };
        let Some(actor) = actor else {
            self.storage
                .finish_participant_change(
                    &row.session_id,
                    &row.id,
                    Some("The effort target runtime is no longer available; nothing was sent."),
                )
                .await?;
            let same_parent = self
                .sessions
                .lock()
                .await
                .get(&row.session_id)
                .is_some_and(|h| context.matches(h));
            if same_parent {
                self.change_notice(&row.session_id, "Effort change failed before sending: the target runtime is no longer available.").await;
                boundary.release().await.map_err(anyhow::Error::msg)?;
            } else {
                boundary.shutdown().await;
            }
            return Ok(());
        };
        context.hold_watchdog().await;
        let mut observed = None;
        let mut record = false;
        let mut quarantine = false;
        let result: Result<()> = async {
            self.check_change_owner(&row.session_id, run, context)
                .await?;
            anyhow::ensure!(
                context.activity.pause_generation() == pause && !context.activity.holds_wakes(),
                "session was paused before the effort change"
            );
            let before = match actor.1.apply(&desired).await {
                Ok(before) => before,
                Err(failure) => {
                    record = failure.may_have_changed || failure.current.is_some();
                    quarantine = !failure.restored;
                    observed = failure.current.clone();
                    return Err(failure.into());
                }
            };
            let publication: Result<()> = async {
                let _lifecycle = self.publication_guard(&row.session_id, run).await?;
                let sessions = self.sessions.lock().await;
                anyhow::ensure!(
                    sessions
                        .get(&row.session_id)
                        .is_some_and(|h| Self::owns_change(h, context, run)),
                    "effort change lost its runtime before publication"
                );
                let mut tx = self.storage.pool().begin_with("BEGIN IMMEDIATE").await?;
                if let Some(claim) = claim {
                    claim.reverify_on(&mut tx).await?;
                    claim.check(&self.bridge)?;
                }
                anyhow::ensure!(
                    context.activity.pause_generation() == pause
                        && !context.activity.holds_wakes()
                        && !run.is_cancelled(),
                    "session was paused or cancelled during effort change"
                );
                anyhow::ensure!(
                    Storage::publish_participant_effort_on(&mut tx, &row.session_id, &row.id)
                        .await?,
                    "effort publication lost to closure or recovery"
                );
                if let Some(claim) = claim {
                    claim.check(&self.bridge)?;
                }
                anyhow::ensure!(
                    context.activity.pause_generation() == pause
                        && !context.activity.holds_wakes()
                        && !run.is_cancelled(),
                    "session was paused or cancelled before effort publication"
                );
                tx.commit().await?;
                Ok(())
            }
            .await;
            if let Err(error) = publication {
                record = true;
                match actor.1.apply(&before).await {
                    Ok(_) => observed = Some(before),
                    Err(failure) => {
                        observed = failure.current;
                        quarantine = true;
                    }
                }
                return Err(error);
            }
            Ok(())
        }
        .await;
        if let Err(error) = &result {
            let same_parent = self
                .sessions
                .lock()
                .await
                .get(&row.session_id)
                .is_some_and(|h| context.matches(h));
            if same_parent {
                if quarantine {
                    actor.2.kill();
                }
                // No fresh model/helper work may consume an unconfirmed pair.
                let mut tx = self.storage.pool().begin_with("BEGIN IMMEDIATE").await?;
                let reason = crate::policy::secret_scan::redact_string(error.to_string());
                Storage::fail_participant_effort_on(
                    &mut tx,
                    &row.session_id,
                    &row.id,
                    record,
                    if quarantine { None } else { observed.as_ref() },
                    &reason,
                )
                .await?;
                tx.commit().await?;
                if quarantine {
                    self.change_recap(&row.session_id, "Effort could not be confirmed or restored. The affected runtime was stopped; restart its agents to resume the saved conversation safely.").await;
                } else {
                    self.change_notice(&row.session_id, &format!("Effort change failed: {reason}. The previous runtime remains in place.")).await;
                }
            } else {
                boundary.shutdown().await;
                return result;
            }
        } else {
            self.change_notice(
                &row.session_id,
                &format!(
                    "{} now uses {}{} effort in the same conversation.",
                    actor.0,
                    desired.effort,
                    if desired.ultracode {
                        " + Ultracode"
                    } else {
                        ""
                    }
                ),
            )
            .await;
        }
        let roster = self
            .storage
            .participants_for_session(&row.session_id)
            .await?;
        {
            let mut sessions = self.sessions.lock().await;
            if let Some(handle) = sessions
                .get_mut(&row.session_id)
                .filter(|h| context.matches(h))
            {
                refresh_runtime_roster(handle, &self.storage, &self.bridge, &roster);
            } else {
                drop(sessions);
                boundary.shutdown().await;
                return result;
            }
        }
        self.bridge
            .notify_participant_effort_changed(&row.session_id, &actor.0);
        boundary.release().await.map_err(anyhow::Error::msg)?;
        result
    }
}
