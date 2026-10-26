//! In-flight direct commands belong to their issuer's runtime and turn.
//! This is cancellation plumbing, not an approval/run ledger.
use super::{
    action_gate::{format_command_output, GatedRun},
    *,
};
use crate::agents::incarnation::{CommandStop, CommandTicket, Incarnation};
use crate::policy::tool_gate;
use crate::storage::MessageKind;
use anyhow::Context;
use std::{
    io,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Duration,
};
use tokio::sync::{oneshot, watch};

pub(super) const INTERRUPT_GRACE: Duration = Duration::from_secs(8);

struct RequestLifetime {
    alive: Arc<AtomicBool>,
    _closed: oneshot::Sender<()>,
}
impl Drop for RequestLifetime {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct SpawnBarrier {
    pub entered: tokio::sync::Notify,
    pub release: tokio::sync::Notify,
    pub stopped: tokio::sync::Notify,
}

pub(super) struct DirectContext {
    bridge: Arc<SignalingBridge>,
    session: String,
    agent: String,
    proof: McpTokenProof,
    owner: Arc<Incarnation>,
    ticket: CommandTicket,
    activity: Arc<ActivityTracker>,
    pause: u64,
    epoch: Arc<AtomicU64>,
    expected_epoch: u64,
}

impl DirectContext {
    fn check(&self) -> io::Result<()> {
        if !self.owner.is_active()
            || self.activity.holds_wakes()
            || self.activity.pause_generation() != self.pause
            || self.epoch.load(Ordering::Acquire) != self.expected_epoch
            || !self
                .bridge
                .token_proof_is_current(&self.session, &self.agent, &self.proof)
        {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "issuer stopped or changed before command spawn; NOT RUN",
            ));
        }
        Ok(())
    }

    async fn stopped(
        &self,
        stopped: &mut watch::Receiver<Option<CommandStop>>,
        lost: &mut oneshot::Receiver<()>,
    ) -> CommandStop {
        loop {
            if let Some(stop) = stopped.borrow().clone() {
                return stop;
            }
            if !self
                .bridge
                .token_proof_is_current(&self.session, &self.agent, &self.proof)
            {
                return CommandStop {
                    reason: "issuer credential revoked".into(),
                    at: tokio::time::Instant::now(),
                };
            }
            tokio::select! {
                _=stopped.changed()=>{},
                _=&mut *lost=>return CommandStop {reason:"caller disconnected".into(),at:tokio::time::Instant::now()},
                _=tokio::time::sleep(Duration::from_millis(100))=>{},
            }
        }
    }
}

fn needs_grace(command: &str) -> bool {
    crate::core::pump::is_atomic_command("action_gate", &serde_json::json!({"command":command}))
        || is_outward(command)
}

fn excerpt(text: &str, limit: usize) -> String {
    // Redact BEFORE truncation, which could otherwise split a credential.
    let text = crate::policy::secret_scan::redact(text);
    if text.len() <= limit {
        return text.into_owned();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[truncated]", &text[..end])
}

impl SignalingBridge {
    #[cfg(test)]
    pub(crate) fn block_direct_spawn(&self) -> Arc<SpawnBarrier> {
        let barrier = Arc::new(SpawnBarrier::default());
        *self.direct_spawn_barrier.lock().unwrap() = Some(barrier.clone());
        barrier
    }
    pub(super) async fn direct_command_context(
        &self,
        session: &str,
        agent: &str,
        proof: Option<&McpTokenProof>,
    ) -> Result<Option<DirectContext>> {
        // Internal bridge callers/tests have no ingress credential. Every
        // real MCP dispatch passes the proof captured by its HTTP handler.
        let Some(proof) = proof else {
            return Ok(None);
        };
        let core = self
            .app_state
            .get()
            .and_then(Weak::upgrade)
            .context("direct-command runtime is unavailable; NOT RUN")?;
        let sessions = core.sessions.lock().await;
        let live = sessions
            .get(session)
            .context("issuer session is not running; NOT RUN")?;
        let parent = live
            .agents()
            .find(|p| p.slug == agent)
            .context("issuer runtime is missing; NOT RUN")?;
        let epoch = parent
            .turn_epoch
            .clone()
            .context("issuer has no turn identity; NOT RUN")?;
        let expected_epoch = epoch.load(Ordering::Acquire);
        let owner = parent.handle.incarnation();
        let ticket = owner
            .command_ticket(expected_epoch)
            .context("issuer has no current turn, or that turn ended; NOT RUN")?;
        let context = DirectContext {
            bridge: core.bridge.clone(),
            session: session.into(),
            agent: agent.into(),
            proof: proof.clone(),
            owner,
            ticket,
            activity: live.activity.clone(),
            pause: live.activity.pause_generation(),
            epoch,
            expected_epoch,
        };
        context.check()?;
        Ok(Some(context))
    }

    pub(super) async fn execute_direct_command(
        &self,
        session: &str,
        command: &str,
        timeout: Duration,
        context: Option<DirectContext>,
    ) -> Result<GatedRun> {
        let Some(context) = context else {
            return self
                .execute_gated_with(session, command, timeout, &[])
                .await;
        };
        let (registration, mut stopped) = context
            .owner
            .attach_command(context.ticket)
            .context("issuer interrupted before command registration; NOT RUN")?;
        // Subscribe/register BEFORE the final checks. The guard below holds
        // the incarnation's command lock across those checks and cmd.spawn.
        context.check()?;
        let command = command.to_string();
        let (request, mut lost) = oneshot::channel::<()>();
        let alive = Arc::new(AtomicBool::new(true));
        let request = RequestLifetime {
            alive: alive.clone(),
            _closed: request,
        };
        let job = tokio::spawn(async move {
            #[cfg(test)]
            let barrier = context.bridge.direct_spawn_barrier.lock().unwrap().clone();
            let began = tokio::time::Instant::now();
            let started = AtomicBool::new(false);
            let stopping = AtomicBool::new(false);
            let spawn = |cmd: &mut tokio::process::Command| {
                registration.start(|| {
                    if !alive.load(Ordering::Acquire) || stopping.load(Ordering::Acquire) {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "caller or issuer stopped before spawn; NOT RUN",
                        ));
                    }
                    context.check()?;
                    let child = cmd.spawn()?;
                    started.store(true, Ordering::Release);
                    Ok(child)
                })
            };
            let execute = async {
                #[cfg(test)]
                if let Some(barrier) = barrier.as_ref() {
                    barrier.entered.notify_one();
                    barrier.release.notified().await;
                }
                let cwd = context
                    .bridge
                    .session_working_repo(&context.session)
                    .await
                    .context("session has no working repository; NOT RUN")?;
                let envs = tool_gate::session_envs(&context.session);
                let envs: Vec<_> = envs.iter().map(|(k, v)| (*k, v.as_str())).collect();
                let out =
                    tool_gate::run_in_repo_with_spawn(&command, &cwd, timeout, &envs, Some(&spawn))
                        .await;
                Ok::<_, anyhow::Error>(GatedRun {
                    text: format_command_output(&out),
                    exit_code: out.code,
                    stdout: out.stdout,
                })
            };
            let (result, stop) = {
                tokio::pin!(execute);
                let first = tokio::select! {
                    biased;
                    result=&mut execute=>Ok(result),
                    stop=context.stopped(&mut stopped,&mut lost)=>Err(stop),
                };
                match first {
                    Ok(result) => (result, None),
                    Err(stop) => {
                        stopping.store(true, Ordering::Release);
                        #[cfg(test)]
                        if let Some(barrier) = barrier.as_ref() {
                            barrier.stopped.notify_one();
                        }
                        let deadline = stop.at
                            + if started.load(Ordering::Acquire) && needs_grace(&command) {
                                INTERRUPT_GRACE
                            } else {
                                Duration::ZERO
                            };
                        let result = tokio::select! {
                            biased;
                            result=&mut execute=>result,
                            _=tokio::time::sleep_until(deadline)=>Err(anyhow::anyhow!(
                                "{} after {:.2}s ({}); {}",
                                if started.load(Ordering::Acquire) {"Command interrupted"} else {"Command NOT RUN"},
                                began.elapsed().as_secs_f64(),stop.reason,
                                if started.load(Ordering::Acquire) {"outcome unknown; command cleanup requested. Check the outcome before retrying"} else {"issuer stopped before spawn"})),
                        };
                        (result, Some(stop))
                    }
                }
            }; // Dropping execute triggers the existing group-cleanup path.
               // Completion may win the select at the same instant as a stop.
               // Retain that stop's result, but do not broadcast routine output.
            let stop = stop.or_else(|| stopped.borrow().clone()).or_else(|| {
                (!alive.load(Ordering::Acquire)).then(|| CommandStop {
                    reason: "caller disconnected".into(),
                    at: tokio::time::Instant::now(),
                })
            });
            let reason = stop
                .as_ref()
                .map(|s| format!("; interrupt: {}", s.reason))
                .unwrap_or_default();
            let body = match &result {
                Ok(run) if stop.is_some() => format!(
                    "exit {} after {:.2}s{reason}\n{}",
                    run.exit_code,
                    began.elapsed().as_secs_f64(),
                    excerpt(&run.text, 8192)
                ),
                Ok(run) => format!(
                    "exit {} after {:.2}s; output omitted from this notice",
                    run.exit_code,
                    began.elapsed().as_secs_f64()
                ),
                Err(error) => excerpt(&format!("{error:#}"), 8192),
            };
            let notice = format!(
                "Direct command result for {} (runtime {}): {} — {body}",
                context.agent,
                &context.owner.id()[..8],
                serde_json::to_string(&excerpt(&command, 512)).expect("string encoding")
            );
            if let Some(storage) = context.bridge.storage_handle().await {
                crate::core::post_system_notice(
                    &storage,
                    Some(&context.bridge),
                    &context.session,
                    MessageKind::SystemNotice,
                    notice,
                    None,
                )
                .await;
            }
            // A one-line ordinary receipt covers success just before an
            // interrupt, without relaying untrusted output to every reader.
            result
        });
        let result = job.await.context("direct command supervisor failed")?;
        drop(request);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_and_outward_commands_keep_the_interrupt_grace() {
        for command in [
            "git push origin main",
            "git commit -m x",
            "php artisan migrate",
            "gh pr create --body x",
            "curl -X POST http://example.invalid",
        ] {
            assert!(needs_grace(command), "{command}");
        }
        assert!(!needs_grace("sleep 70"));
    }
}
