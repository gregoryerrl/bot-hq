//! The bot-hq window as a shared resource (feedback #109, Plan Batch 6 B):
//! result-returning evaluation, and the lease that keeps two callers' actions
//! from interleaving. The contract is in `signaling::webview_js`.
use super::*;
use crate::signaling::webview_js::{self, EvalOutcome, LeaseHolder, LeaseStamp};
use serde_json::{json, Value};
use std::time::{Duration, Instant, SystemTime};

/// An evaluation already handed to the main thread (or to a test evaluator),
/// whose result is awaited OUTSIDE the lease lock.
pub(crate) struct QueuedEval {
    rx: Option<tokio::sync::oneshot::Receiver<EvalOutcome>>,
    ready: Option<EvalOutcome>,
}

impl QueuedEval {
    pub(crate) async fn wait(self) -> EvalOutcome {
        if let Some(ready) = self.ready {
            return ready;
        }
        let Some(rx) = self.rx else { return EvalOutcome::NoWebview };
        match tokio::time::timeout(webview_js::EVAL_TIMEOUT, rx).await {
            Ok(Ok(outcome)) => outcome,
            _ => EvalOutcome::Timeout,
        }
    }
}

impl SignalingBridge {
    /// Hand `js` to the main window (`eval_with_callback` on the main thread)
    /// and return at once; the outcome is awaited by [`QueuedEval::wait`].
    /// Queueing is what the lease lock serializes: two evals queued in order
    /// run in that order, whatever their callers do next.
    pub(crate) fn queue_webview_eval(&self, js: String) -> QueuedEval {
        #[cfg(test)]
        if let Some(fake) = self.webview_eval_override.lock().unwrap_or_else(|p| p.into_inner()).clone() {
            return QueuedEval { rx: None, ready: Some(fake(js)) };
        }
        let Some(handle) = self.app_handle() else {
            return QueuedEval { rx: None, ready: Some(EvalOutcome::NoWebview) };
        };
        use tauri::Manager;
        let (tx, rx) = tokio::sync::oneshot::channel::<EvalOutcome>();
        let slot = Arc::new(std::sync::Mutex::new(Some(tx)));
        let handle_inner = handle.clone();
        let slot_cb = Arc::clone(&slot);
        let scheduled = handle.run_on_main_thread(move || {
            let send = |outcome: EvalOutcome| {
                if let Some(tx) = slot_cb.lock().unwrap_or_else(|p| p.into_inner()).take() {
                    let _ = tx.send(outcome);
                }
            };
            match handle_inner.get_webview_window("main") {
                Some(window) => {
                    let slot_result = Arc::clone(&slot_cb);
                    let queued = window.eval_with_callback(js, move |json| {
                        if let Some(tx) = slot_result.lock().unwrap_or_else(|p| p.into_inner()).take() {
                            let outcome = match serde_json::from_str::<serde_json::Value>(&json) {
                                Ok(value) => EvalOutcome::Returned(webview_js::normalize_result(value)),
                                Err(_) => EvalOutcome::Returned(json!({"outcome": "unknown", "ok": false, "matched": null, "reason": "unreadable_result", "detail": json})),
                            };
                            let _ = tx.send(outcome);
                        }
                    });
                    if queued.is_err() {
                        send(EvalOutcome::NoWebview);
                    }
                }
                None => send(EvalOutcome::NoWebview),
            }
        });
        if scheduled.is_err() {
            return QueuedEval { rx: None, ready: Some(EvalOutcome::NoWebview) };
        }
        QueuedEval { rx: Some(rx), ready: None }
    }

    pub(crate) async fn webview_eval(&self, js: String) -> EvalOutcome {
        self.queue_webview_eval(js).wait().await
    }

    #[cfg(test)]
    pub(crate) fn override_webview_eval(&self, fake: webview_js::EvalOverride) {
        *self.webview_eval_override.lock().unwrap_or_else(|p| p.into_inner()) = Some(fake);
    }

    fn holder_is_live(&self, holder: &LeaseHolder, now: Instant) -> bool {
        holder.until > now && self.token_proof_is_current(&holder.session_id, &holder.agent, &holder.proof)
    }

    fn caller_is_holder(&self, holder: &LeaseHolder, session: &str, agent: &str, proof: Option<&McpTokenProof>) -> bool {
        holder.session_id == session
            && holder.agent == agent
            && proof.is_some_and(|p| self.token_proof_is_current(session, agent, p))
    }

    fn lease_refusal(holder: &LeaseHolder) -> Value {
        json!({"outcome": "refused", "ok": false, "matched": false, "reason": "leased",
            "holder": {"session_id": holder.session_id, "agent": holder.agent}, "until_ms": holder.until_ms, "generation": holder.generation,
            "detail": "another caller holds the webview lease; wait for it to expire or be released — nothing was queued"})
    }

    /// Admission for a mutating `webview_*` tool: the stamp its snippet must
    /// carry, or a refusal. Nothing is queued on refusal. The CALLER's own
    /// credential must be the current one — a request authenticated before
    /// its runtime was replaced is retired work, whatever the lease says
    /// (eyes-2, b7982bbd) — and a live lease admits only its holder.
    pub(crate) fn webview_admit(&self, session: &str, agent: &str, proof: Option<&McpTokenProof>) -> Result<LeaseStamp, Value> {
        let Some(proof) = proof else {
            return Err(json!({"outcome": "refused", "ok": false, "matched": false, "reason": "unauthenticated",
                "detail": "a webview action needs the authenticated HTTP caller"}));
        };
        let state = self.webview_lease.lock().unwrap_or_else(|p| p.into_inner());
        // Under the lock: a replacement bumps the generation under this same
        // lock, so a caller found current here is stamped with a generation
        // that replacement will move.
        if !self.token_proof_is_current(session, agent, proof) {
            return Err(json!({"outcome": "refused", "ok": false, "matched": false, "reason": "stale_credential",
                "detail": "this request's credential is no longer the current one for the participant (its runtime was replaced); nothing was queued"}));
        }
        let now = Instant::now();
        match state.holder.as_ref().filter(|h| self.holder_is_live(h, now)) {
            None => Ok(LeaseStamp { generation: state.generation, until_ms: None }),
            Some(h) if self.caller_is_holder(h, session, agent, Some(proof)) => Ok(LeaseStamp { generation: state.generation, until_ms: Some(h.until_ms) }),
            Some(h) => Err(Self::lease_refusal(h)),
        }
    }

    /// A participant's credential changed (its runtime was replaced, or it
    /// retired): every action admitted under the old credential is retired
    /// work. Move the generation and mirror it, so such an action still in the
    /// main-thread queue refuses `lease_lost` at actuation; drop the lease if
    /// that participant held it UNDER THE RETIRED credential — a successor
    /// that already holds it under the new one keeps it (eyes-2, c42ac1e9).
    /// Production reaches this through the registry's one-transition path
    /// (`credential_changed_locked`); this entry stands in for a cleanup that
    /// arrives LATE — after a successor already holds the lease — in tests.
    #[cfg(test)]
    pub(crate) fn webview_credential_changed(&self, session: &str, agent: &str) {
        let mut state = self.webview_lease.lock().unwrap_or_else(|p| p.into_inner());
        self.credential_changed_locked(&mut state, session, agent);
    }

    /// [`Self::webview_credential_changed`] for a caller that already holds the
    /// lease lock — the token registry, which changes the token and the lease
    /// as ONE transition (lock order: lease, then tokens).
    pub(crate) fn credential_changed_locked(&self, state: &mut webview_js::LeaseState, session: &str, agent: &str) {
        let retired_holder = state.holder.as_ref().is_some_and(|h| {
            h.session_id == session && h.agent == agent && !self.token_proof_is_current(session, agent, &h.proof)
        });
        if retired_holder {
            state.holder = None;
        }
        state.generation += 1;
        drop(self.queue_webview_eval(webview_js::mirror_js(state.generation)));
    }

    /// The window was reloaded (its mirror reads 0 while the host's generation
    /// is higher): publish the current generation again so the next action
    /// is admitted against it. Returns whether the window acknowledged it.
    pub(crate) async fn webview_republish_generation(&self) -> bool {
        let (generation, queued) = {
            let state = self.webview_lease.lock().unwrap_or_else(|p| p.into_inner());
            (state.generation, self.queue_webview_eval(webview_js::mirror_js(state.generation)))
        };
        matches!(queued.wait().await, EvalOutcome::Returned(v) if v["gen"].as_u64() == Some(generation))
    }

    pub(crate) fn webview_generation(&self) -> u64 {
        self.webview_lease.lock().unwrap_or_else(|p| p.into_inner()).generation
    }

    /// `webview_lease`: take (or renew) the lease for `ttl`. The generation
    /// moves and its mirror is queued under the lock; the reply waits for the
    /// window's acknowledgement, after which every action queued before the
    /// transfer has run.
    pub(crate) async fn webview_lease_acquire(&self, session: &str, agent: &str, proof: Option<&McpTokenProof>, ttl: Duration) -> Value {
        let Some(proof) = proof else {
            return json!({"outcome": "refused", "ok": false, "reason": "unauthenticated", "detail": "a lease needs the authenticated HTTP caller"});
        };
        let ttl = ttl.min(webview_js::LEASE_MAX_TTL).max(Duration::from_secs(1));
        let (generation, until_ms, renewed, queued) = {
            let mut state = self.webview_lease.lock().unwrap_or_else(|p| p.into_inner());
            // Under the lease lock, like the token registry's own transition:
            // a replacement either precedes this check (stale, refused) or
            // follows the install (and then clears this retired holder).
            if !self.token_proof_is_current(session, agent, proof) {
                return json!({"outcome": "refused", "ok": false, "reason": "stale_credential", "detail": "this request's credential is no longer the current one for the participant (its runtime was replaced)"});
            }
            let now = Instant::now();
            let renewed = match state.holder.as_ref().filter(|h| self.holder_is_live(h, now)) {
                Some(h) if self.caller_is_holder(h, session, agent, Some(proof)) => true,
                Some(h) => return Self::lease_refusal(h),
                None => false,
            };
            state.generation += 1;
            let until_ms = webview_js::epoch_ms(SystemTime::now() + ttl);
            state.holder = Some(LeaseHolder {
                session_id: session.to_string(),
                agent: agent.to_string(),
                proof: proof.clone(),
                until: now + ttl,
                until_ms,
                generation: state.generation,
            });
            let queued = self.queue_webview_eval(webview_js::mirror_js(state.generation));
            (state.generation, until_ms, renewed, queued)
        };
        let mirror = queued.wait().await;
        let established = matches!(&mirror, EvalOutcome::Returned(v) if v["gen"].as_u64() == Some(generation));
        json!({"outcome": "done", "ok": true, "status": if renewed { "renewed" } else { "acquired" }, "generation": generation, "until_ms": until_ms, "ttl_secs": ttl.as_secs(),
            "established": established,
            "mirror": match mirror { EvalOutcome::Returned(v) => v, other => other.into_reply("lease transfer") },
            "note": if established { "the window acknowledged the new generation: every action queued before this transfer has run; your actions are admitted until the deadline and refused at actuation after it" } else { "the window has not acknowledged the new generation; your actions are refused (lease_lost) until it does — acquire again, or read the window first" }})
    }

    /// `webview_release`: give the lease up. Only the holder can; the
    /// generation moves so a late action of the holder's refuses.
    pub(crate) async fn webview_lease_release(&self, session: &str, agent: &str, proof: Option<&McpTokenProof>) -> Value {
        let Some(proof) = proof else {
            return json!({"outcome": "refused", "ok": false, "reason": "unauthenticated", "detail": "a release needs the authenticated HTTP caller"});
        };
        let proof = Some(proof);
        let (generation, queued) = {
            let mut state = self.webview_lease.lock().unwrap_or_else(|p| p.into_inner());
            if !self.token_proof_is_current(session, agent, proof.unwrap()) {
                return json!({"outcome": "refused", "ok": false, "reason": "stale_credential", "detail": "this request's credential is no longer the current one for the participant (its runtime was replaced)"});
            }
            let now = Instant::now();
            match state.holder.as_ref() {
                Some(h) if self.holder_is_live(h, now) && !self.caller_is_holder(h, session, agent, proof) => {
                    return json!({"outcome": "refused", "ok": false, "reason": "not_holder", "holder": {"session_id": h.session_id, "agent": h.agent}, "until_ms": h.until_ms});
                }
                Some(_) => {}
                None => return json!({"outcome": "done", "ok": true, "status": "free", "generation": state.generation}),
            }
            state.holder = None;
            state.generation += 1;
            let queued = self.queue_webview_eval(webview_js::mirror_js(state.generation));
            (state.generation, queued)
        };
        let mirror = queued.wait().await;
        json!({"outcome": "done", "ok": true, "status": "released", "generation": generation,
            "mirror": match mirror { EvalOutcome::Returned(v) => v, other => other.into_reply("lease release") }})
    }

    /// The window's route and session, sampled by a separate evaluation —
    /// metadata beside a screenshot, not the frame's atomic context.
    pub(crate) async fn webview_context(&self) -> Value {
        match self.webview_eval(webview_js::context_js()).await {
            EvalOutcome::Returned(v) => json!({"route": v["route"], "session_id": v["session_id"], "lease_gen": v["lease_gen"], "sampled": "before capture"}),
            other => json!({"sampled": "unavailable", "why": other.into_reply("context read")}),
        }
    }
}
