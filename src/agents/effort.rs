//! Process-scoped effort control. A successful settings ACK is not readback.
//! Claude replies bypass the model event stream; Codex reads its shared pair
//! only when assembling the next turn/start request.
use super::protocol::ControlRequest;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicI8, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

pub const LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// Keep diagnostics to this one non-secret enum, never a dump of settings/env.
fn settings_pin(settings: &Value) -> Option<String> {
    let value = settings["effective"]["env"]["CLAUDE_CODE_EFFORT_LEVEL"].as_str()?;
    if value.trim().is_empty() {
        return None;
    }
    let source = settings["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .rev()
        .find(|s| s["settings"]["env"]["CLAUDE_CODE_EFFORT_LEVEL"].as_str() == Some(value))
        .and_then(|s| s["source"].as_str())
        .filter(|s| {
            matches!(
                *s,
                "userSettings"
                    | "projectSettings"
                    | "localSettings"
                    | "flagSettings"
                    | "policySettings"
                    | "managedSettings"
            )
        })
        .unwrap_or("settings");
    let value = if LEVELS.contains(&value) {
        value
    } else {
        "<set>"
    };
    Some(format!("{source}.env.CLAUDE_CODE_EFFORT_LEVEL={value}"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EffortPair {
    pub effort: String,
    pub ultracode: bool,
}

/// Failure includes the final readback after a best-effort restore. `None`
/// means the caller must not retain an earlier pair as confirmed.
#[derive(Debug)]
pub struct EffortFailure {
    pub current: Option<EffortPair>,
    pub may_have_changed: bool,
    pub restored: bool,
    reason: String,
}
impl std::fmt::Display for EffortFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}
impl std::error::Error for EffortFailure {}
impl EffortPair {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            LEVELS.contains(&self.effort.as_str()),
            "unknown effort level"
        );
        anyhow::ensure!(
            !self.ultracode || self.effort == "xhigh",
            "Ultracode requires xhigh effort"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct EffortStatus {
    pub available: bool,
    pub reason: Option<String>,
    pub current: Option<EffortPair>,
    pub ultracode_available: bool,
}
impl EffortStatus {
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            available: false,
            reason: Some(reason.into()),
            current: None,
            ultracode_available: false,
        }
    }
}

/// The stdout reader and the request client share only correlation state.
/// No response ever binds a turn, counts as activity, or consumes a cursor.
#[derive(Clone)]
pub(crate) struct ClaudeReplies(Arc<ReplyState>);
struct ReplyState {
    pending: Mutex<HashMap<String, oneshot::Sender<Value>>>,
    capability: AtomicI8, // -1 unknown, 0 unavailable, 1 active
    initialized: AtomicBool,
    closed: AtomicBool,
    revision: AtomicU64,
}
impl ClaudeReplies {
    pub fn new() -> Self {
        Self(Arc::new(ReplyState {
            pending: Mutex::new(HashMap::new()),
            capability: AtomicI8::new(-1),
            initialized: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            revision: AtomicU64::new(0),
        }))
    }
    pub fn observe(&self, value: &Value) {
        if value["type"] == "control_response" {
            if let Some(id) = value["response"]["request_id"].as_str() {
                if let Some(tx) = self
                    .0
                    .pending
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(id)
                {
                    let _ = tx.send(value["response"].clone());
                }
            }
        } else if value["type"] == "system" && value["subtype"] == "init" {
            if !self.0.initialized.swap(true, Ordering::AcqRel) {
                self.0.revision.fetch_add(1, Ordering::AcqRel);
            }
            self.set_capability(match value["per_turn_effort_active"].as_bool() {
                Some(true) => 1,
                Some(false) => 0,
                None => -1,
            });
        } else if value["type"] == "system" && value["subtype"] == "per_turn_effort_changed" {
            // This notice reports whether the per-message mechanism is active,
            // not a confirmation that a selected effort level took effect.
            self.set_capability(0);
        }
    }
    fn set_capability(&self, capability: i8) {
        if self.0.capability.swap(capability, Ordering::AcqRel) != capability {
            self.0.revision.fetch_add(1, Ordering::AcqRel);
        }
    }
    pub fn close(&self) {
        self.0.closed.store(true, Ordering::Release);
        self.0.revision.fetch_add(1, Ordering::AcqRel);
        self.0
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }
    async fn request(&self, tx: &mpsc::Sender<ControlRequest>, body: Value) -> Result<Value> {
        anyhow::ensure!(
            !self.0.closed.load(Ordering::Acquire),
            "agent settings stream is closed"
        );
        let id = uuid::Uuid::new_v4().to_string();
        let (reply, rx) = oneshot::channel();
        self.0
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.clone(), reply);
        struct Pending(ClaudeReplies, String);
        impl Drop for Pending {
            fn drop(&mut self) {
                self.0
                     .0
                    .pending
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&self.1);
            }
        }
        let _pending = Pending(self.clone(), id.clone());
        let response = tokio::time::timeout(Duration::from_secs(5), async {
            tx.send(ControlRequest::settings(id, body))
                .await
                .context("agent settings channel closed")?;
            rx.await.context("agent closed before settings readback")
        })
        .await
        .context("agent settings request timed out; outcome is unconfirmed")??;
        // Never echo a native error body: it may contain user settings/secrets.
        anyhow::ensure!(
            response["subtype"] == "success",
            "agent refused the settings request"
        );
        Ok(response.get("response").cloned().unwrap_or(Value::Null))
    }
}

#[derive(Clone)]
enum Backend {
    Unavailable(String),
    Claude {
        replies: ClaudeReplies,
        tx: mpsc::Sender<ControlRequest>,
        gateway: bool,
    },
    Codex {
        pair: Arc<Mutex<EffortPair>>,
        alive: Arc<AtomicBool>,
    },
}

/// Clones follow automatic same-conversation respawns. The lock serializes
/// updates/readback with replacement; confirmed picks also seed retry config.
#[derive(Clone)]
pub struct EffortControl {
    backend: Arc<tokio::sync::Mutex<Backend>>,
    retry: Arc<Mutex<Option<EffortPair>>>,
    cache: Arc<Mutex<Option<(u64, EffortStatus, tokio::time::Instant)>>>,
}
impl Default for EffortControl {
    fn default() -> Self {
        Self::new(Backend::Unavailable(
            "Live effort is unavailable for this runtime".into(),
        ))
    }
}
impl EffortControl {
    fn new(backend: Backend) -> Self {
        Self {
            backend: Arc::new(tokio::sync::Mutex::new(backend)),
            retry: Arc::new(Mutex::new(None)),
            cache: Arc::new(Mutex::new(None)),
        }
    }
    pub(crate) fn claude(
        replies: ClaudeReplies,
        tx: mpsc::Sender<ControlRequest>,
        gateway: bool,
    ) -> Self {
        Self::new(Backend::Claude {
            replies,
            tx,
            gateway,
        })
    }
    pub(crate) fn codex(pair: Arc<Mutex<EffortPair>>) -> Self {
        Self::new(Backend::Codex {
            pair,
            alive: Arc::new(AtomicBool::new(true)),
        })
    }
    pub(crate) async fn replace_runtime(&self, next: &Self) {
        let backend = next.backend.lock().await.clone();
        *self.backend.lock().await = backend;
        *self.cache.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
    pub(crate) async fn disconnected(&self) {
        let mut backend = self.backend.lock().await;
        if let Backend::Codex { alive, .. } = &*backend {
            alive.store(false, Ordering::Release);
        }
        *backend = Backend::Unavailable("The agent is restarting or has stopped".into());
        *self.cache.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
    pub(crate) fn retry_pair(&self) -> Option<EffortPair> {
        self.retry.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Observation does not require system/init; the CLI can answer this before
    /// its first model turn. Capability is checked separately for mutations.
    async fn read(backend: &Backend) -> Result<(EffortPair, bool, Option<String>)> {
        match backend {
            Backend::Unavailable(reason) => anyhow::bail!("{reason}"),
            Backend::Claude { replies, tx, .. } => {
                let settings = replies
                    .request(tx, json!({"subtype":"get_settings"}))
                    .await?;
                let applied = &settings["applied"];
                let pair = EffortPair {
                    effort: applied["effort"]
                        .as_str()
                        .context("agent did not report applied effort")?
                        .to_string(),
                    ultracode: applied["ultracode"]
                        .as_bool()
                        .context("agent did not report applied Ultracode")?,
                };
                pair.validate()?;
                Ok((
                    pair,
                    applied["ultracodeAvailable"].as_bool().unwrap_or(false),
                    settings_pin(&settings),
                ))
            }
            Backend::Codex { pair, alive } => {
                anyhow::ensure!(alive.load(Ordering::Acquire), "Codex runtime has stopped");
                let pair = pair.lock().unwrap_or_else(|p| p.into_inner()).clone();
                pair.validate()?;
                Ok((pair, false, None))
            }
        }
    }

    fn mutation_reason(backend: &Backend, pin: Option<&str>) -> Option<String> {
        match backend {
            Backend::Unavailable(reason) => Some(reason.clone()),
            Backend::Claude { gateway: true, .. } => Some("Live effort has not been verified for this gateway".into()),
            Backend::Claude { replies, .. } if !replies.0.initialized.load(Ordering::Acquire) =>
                Some("Live effort changes are available after this agent's first turn since it started, when the runtime reports support.".into()),
            Backend::Claude { replies, .. } if replies.0.capability.load(Ordering::Acquire) != 1 =>
                Some("This runtime does not report active per-message effort support".into()),
            _ => pin.map(|pin| format!("Effort is pinned by {pin}; remove that conflict before changing it")),
        }
    }
    pub async fn status(&self) -> EffortStatus {
        let backend = self.backend.lock().await;
        let revision = match &*backend {
            Backend::Claude { replies, .. } => replies.0.revision.load(Ordering::Acquire),
            Backend::Codex { alive, .. } => u64::from(alive.load(Ordering::Acquire)),
            _ => 0,
        };
        if let Some((stamp, status, at)) = &*self.cache.lock().unwrap_or_else(|p| p.into_inner()) {
            if *stamp == revision && (status.available || at.elapsed() < Duration::from_secs(10)) {
                return status.clone();
            }
        }
        let status = match Self::read(&backend).await {
            Ok((current, ultracode_available, pin)) => EffortStatus {
                available: Self::mutation_reason(&backend, pin.as_deref()).is_none(),
                reason: Self::mutation_reason(&backend, pin.as_deref()),
                current: Some(current),
                ultracode_available,
            },
            Err(e) => EffortStatus::unavailable(
                Self::mutation_reason(&backend, None)
                    .map(|reason| format!("{reason} Current effort is unconfirmed: {e}"))
                    .unwrap_or_else(|| e.to_string()),
            ),
        };
        *self.cache.lock().unwrap_or_else(|p| p.into_inner()) =
            Some((revision, status.clone(), tokio::time::Instant::now()));
        status
    }
    async fn write(backend: &Backend, desired: &EffortPair) -> Result<()> {
        match backend {
            Backend::Claude { replies, tx, .. } => {
                // Native 2.1.308 probes confirm this merges into --settings,
                // preserving hooks/attribution. Never send absent keys as null.
                replies.request(tx, json!({"subtype":"apply_flag_settings", "settings": {"effortLevel": desired.effort, "ultracode": desired.ultracode}})).await?;
            }
            Backend::Codex { pair, .. } => {
                *pair.lock().unwrap_or_else(|p| p.into_inner()) = desired.clone()
            }
            Backend::Unavailable(reason) => anyhow::bail!("{reason}"),
        }
        Ok(())
    }

    /// Returns the old actual pair for publication rollback. After any send
    /// failure/mismatch, restore AND read again even when the restore ACK fails.
    pub async fn apply(
        &self,
        desired: &EffortPair,
    ) -> std::result::Result<EffortPair, EffortFailure> {
        let before_send = |error: anyhow::Error| EffortFailure {
            current: None,
            may_have_changed: false,
            restored: true,
            reason: error.to_string(),
        };
        desired.validate().map_err(before_send)?;
        let backend = self.backend.lock().await;
        *self.cache.lock().unwrap_or_else(|p| p.into_inner()) = None;
        let (before, ultra_available, pin) = Self::read(&backend).await.map_err(before_send)?;
        if let Some(reason) = Self::mutation_reason(&backend, pin.as_deref()) {
            return Err(EffortFailure {
                current: Some(before),
                may_have_changed: false,
                restored: true,
                reason,
            });
        }
        if desired.ultracode && !ultra_available {
            return Err(EffortFailure {
                current: Some(before),
                may_have_changed: false,
                restored: true,
                reason: "Ultracode is unavailable for this agent".into(),
            });
        }
        let result: Result<()> = async {
            Self::write(&backend, desired).await?;
            let (actual, _, pin) = Self::read(&backend).await?;
            anyhow::ensure!(actual == *desired, "Effort readback differs: requested {} (Ultracode {}), observed {} (Ultracode {}). {}",
                desired.effort, desired.ultracode, actual.effort, actual.ultracode,
                pin.as_deref().unwrap_or("Check settings or environment overrides."));
            if let Some(reason) = Self::mutation_reason(&backend, pin.as_deref()) { anyhow::bail!("{reason}"); }
            Ok(())
        }.await;
        if let Err(error) = result {
            let _ = Self::write(&backend, &before).await;
            let current = Self::read(&backend).await.ok().map(|(pair, _, _)| pair);
            *self.retry.lock().unwrap_or_else(|p| p.into_inner()) = current.clone();
            let restored = current.as_ref() == Some(&before);
            return Err(EffortFailure {
                current,
                may_have_changed: true,
                restored,
                reason: error.to_string(),
            });
        }
        *self.retry.lock().unwrap_or_else(|p| p.into_inner()) = Some(desired.clone());
        Ok(before)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(level: &str, ultracode: bool) -> EffortPair {
        EffortPair {
            effort: level.into(),
            ultracode,
        }
    }
    fn fixture(pinned: bool) -> (EffortControl, ClaudeReplies) {
        let (tx, mut rx) = mpsc::channel::<ControlRequest>(8);
        let replies = ClaudeReplies::new();
        let observer = replies.clone();
        tokio::spawn(async move {
            let mut actual = pair("high", false);
            while let Some(request) = rx.recv().await {
                let wire = serde_json::to_value(&request).unwrap();
                let response = match request.request.subtype {
                    "get_settings" => {
                        json!({"effective":{"effortLevel":"medium"},"applied":{"effort":actual.effort,"ultracode":actual.ultracode,"ultracodeAvailable":true}})
                    }
                    "apply_flag_settings" => {
                        let settings = wire["request"]["settings"].as_object().unwrap();
                        assert_eq!(
                            settings.len(),
                            2,
                            "an update must preserve the launch hook/attribution layer"
                        );
                        assert!(
                            settings.contains_key("effortLevel")
                                && settings.contains_key("ultracode")
                        );
                        if !pinned {
                            actual = pair(
                                settings["effortLevel"].as_str().unwrap(),
                                settings["ultracode"].as_bool().unwrap(),
                            );
                        }
                        json!({})
                    }
                    other => panic!("unexpected settings request: {other}"),
                };
                observer.observe(&json!({"type":"control_response","response":{"subtype":"success","request_id":request.request_id,"response":response}}));
            }
        });
        (EffortControl::claude(replies.clone(), tx, false), replies)
    }
    fn init(replies: &ClaudeReplies, active: bool) {
        replies.observe(&json!({"type":"system","subtype":"init","per_turn_effort_active":active}));
    }

    #[tokio::test]
    async fn startup_observes_actual_effort_before_live_capability_arrives() {
        let (control, replies) = fixture(false);
        let initial = control.status().await;
        assert_eq!(initial.current, Some(pair("high", false)));
        assert!(!initial.available);
        assert!(initial
            .reason
            .unwrap()
            .contains("after this agent's first turn"));
        let error = control.apply(&pair("low", false)).await.unwrap_err();
        assert!(!error.may_have_changed);
        assert_eq!(error.current, Some(pair("high", false)));
        init(&replies, true);
        assert!(control.status().await.available);
        control.apply(&pair("low", false)).await.unwrap();
        replies.observe(&json!({"type":"system","subtype":"per_turn_effort_changed","per_turn_effort_active":false}));
        let unsupported = control.status().await;
        assert_eq!(unsupported.current, Some(pair("low", false)));
        assert!(!unsupported.available);
        assert!(!unsupported
            .reason
            .unwrap()
            .contains("after this agent's first turn"));
    }

    #[test]
    fn pin_diagnostic_names_only_the_relevant_source_and_safe_value() {
        let settings = json!({"effective":{"env":{"CLAUDE_CODE_EFFORT_LEVEL":"max","API_SECRET":"canary"}},"sources":[{"source":"userSettings","settings":{"env":{"CLAUDE_CODE_EFFORT_LEVEL":"max"}}}]});
        assert_eq!(
            settings_pin(&settings).as_deref(),
            Some("userSettings.env.CLAUDE_CODE_EFFORT_LEVEL=max")
        );
        let mut unknown = settings;
        unknown["effective"]["env"]["CLAUDE_CODE_EFFORT_LEVEL"] = json!("secret-canary");
        let message = settings_pin(&unknown).unwrap();
        assert!(message.ends_with("=<set>") && !message.contains("canary"));
    }

    #[tokio::test]
    async fn effort_requires_own_capability_and_confirms_both_runtime_knobs() {
        let (control, replies) = fixture(false);
        assert!(!control.status().await.available);
        init(&replies, true);
        assert_eq!(
            control.status().await.current,
            Some(pair("high", false)),
            "layered effective settings are not runtime readback"
        );
        for desired in [pair("xhigh", true), pair("low", false), pair("max", false)] {
            control.apply(&desired).await.unwrap();
            assert_eq!(control.status().await.current, Some(desired.clone()));
            assert_eq!(control.retry_pair(), Some(desired));
        }
        assert!(control.apply(&pair("max", true)).await.is_err());
        replies.observe(&json!({"type":"system","subtype":"per_turn_effort_changed","per_turn_effort_active":false}));
        assert!(!control.status().await.available);
        assert!(control.apply(&pair("low", false)).await.is_err());
    }

    #[tokio::test]
    async fn successful_ack_does_not_hide_a_pinned_effort_or_change_retry_config() {
        let (control, replies) = fixture(true);
        init(&replies, true);
        assert!(control
            .apply(&pair("low", false))
            .await
            .unwrap_err()
            .to_string()
            .contains("readback differs"));
        assert_eq!(control.status().await.current, Some(pair("high", false)));
        assert_eq!(
            control.retry_pair(),
            Some(pair("high", false)),
            "a verified rollback seeds the retry configuration too"
        );
        assert!(
            control.apply(&pair("xhigh", true)).await.is_err(),
            "Ultracode must also read back"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn failed_sent_changes_restore_and_return_the_final_observation() {
        for mode in ["partial", "lost", "unknown", "stuck"] {
            let replies = ClaudeReplies::new();
            init(&replies, true);
            let (tx, mut rx) = mpsc::channel::<ControlRequest>(8);
            let observer = replies.clone();
            tokio::spawn(async move {
                let mut actual = pair("high", false);
                let mut writes = 0;
                while let Some(request) = rx.recv().await {
                    let mut response =
                        json!({"subtype":"success","request_id":request.request_id,"response":{}});
                    if request.request.subtype == "apply_flag_settings" {
                        writes += 1;
                        let settings = &request.request.fields["settings"];
                        let previous = actual.clone();
                        actual = pair(
                            settings["effortLevel"].as_str().unwrap(),
                            settings["ultracode"].as_bool().unwrap(),
                        );
                        if writes == 1 && matches!(mode, "partial" | "stuck") {
                            actual.ultracode = false;
                        }
                        if writes > 1 && mode == "stuck" {
                            actual = previous;
                        }
                        if writes == 1 && mode == "lost" {
                            continue;
                        } // applied, ACK lost
                    } else if mode == "unknown" && writes > 0 {
                        response["subtype"] = json!("error");
                    } else {
                        response["response"] = json!({"applied":{"effort":actual.effort,"ultracode":actual.ultracode,"ultracodeAvailable":true}});
                    }
                    observer.observe(&json!({"type":"control_response","response":response}));
                }
            });
            let control = EffortControl::claude(replies.clone(), tx, false);
            let failure = control.apply(&pair("xhigh", true)).await.unwrap_err();
            assert!(failure.may_have_changed);
            if mode == "stuck" {
                assert!(!failure.restored);
                assert_eq!(failure.current, Some(pair("xhigh", false)));
            } else if mode == "unknown" {
                assert!(!failure.restored);
                assert_eq!(failure.current, None);
                assert_eq!(control.retry_pair(), None);
            } else {
                assert!(failure.restored);
                assert_eq!(failure.current, Some(pair("high", false)));
                assert_eq!(control.retry_pair(), Some(pair("high", false)));
            }
            assert!(replies.0.pending.lock().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn stdout_settings_responses_are_correlated_without_model_events() {
        use tokio::io::AsyncWriteExt;
        let replies = ClaudeReplies::new();
        let (tx, mut rx) = mpsc::channel::<ControlRequest>(8);
        let control = EffortControl::claude(replies.clone(), tx, false);
        let (mut stdout, reader) = tokio::io::duplex(8192);
        let (events, mut event_rx) = mpsc::channel(16);
        let pump = tokio::spawn(super::super::events::pump_events_with_controls(
            reader,
            events,
            Some(replies),
        ));
        stdout
            .write_all(
                b"{\"type\":\"system\",\"subtype\":\"init\",\"per_turn_effort_active\":true}\n",
            )
            .await
            .unwrap();
        assert!(matches!(
            event_rx.recv().await,
            Some(super::super::spawn::AgentEvent::Init { .. })
        ));
        let read = tokio::spawn(async move { control.status().await });
        let request = rx.recv().await.unwrap();
        stdout.write_all(format!("{}\n", json!({"type":"control_response","response":{"subtype":"success","request_id":request.request_id,"response":{"applied":{"effort":"low","ultracode":false}}}})).as_bytes()).await.unwrap();
        assert_eq!(read.await.unwrap().current, Some(pair("low", false)));
        drop(stdout);
        pump.await.unwrap();
        assert!(
            event_rx.recv().await.is_none(),
            "readback is never a model event/turn completion"
        );
    }

    #[tokio::test]
    async fn malformed_readback_and_native_refusal_are_unavailable_without_echoing_native_settings()
    {
        for response in [
            json!({"subtype":"success","response":{"effective":{"effortLevel":"low"}}}),
            json!({"subtype":"error","error":"secret-settings-canary"}),
        ] {
            let replies = ClaudeReplies::new();
            init(&replies, true);
            let (tx, mut rx) = mpsc::channel::<ControlRequest>(8);
            let control = EffortControl::claude(replies.clone(), tx, false);
            let read = tokio::spawn(async move { control.status().await });
            let request = rx.recv().await.unwrap();
            let mut response = response;
            response["request_id"] = json!(request.request_id);
            replies.observe(&json!({"type":"control_response","response":response}));
            let status = read.await.unwrap();
            assert!(!status.available);
            assert!(!status.reason.unwrap().contains("secret-settings-canary"));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn timed_out_cancelled_unknown_and_disconnected_responses_leave_no_pending_ids() {
        let replies = ClaudeReplies::new();
        let (tx, mut rx) = mpsc::channel(8);
        let request = {
            let replies = replies.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                replies
                    .request(&tx, json!({"subtype":"get_settings"}))
                    .await
            })
        };
        let wire = rx.recv().await.unwrap();
        replies.observe(&json!({"type":"control_response","response":{"subtype":"success","request_id":"wrong","response":{}}}));
        assert_eq!(replies.0.pending.lock().unwrap().len(), 1);
        assert!(request
            .await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("timed out"));
        assert!(replies.0.pending.lock().unwrap().is_empty());
        replies.observe(&json!({"type":"control_response","response":{"subtype":"success","request_id":wire.request_id,"response":{}}}));
        let request = {
            let replies = replies.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                replies
                    .request(&tx, json!({"subtype":"get_settings"}))
                    .await
            })
        };
        let _ = rx.recv().await.unwrap();
        request.abort();
        let _ = request.await;
        assert!(replies.0.pending.lock().unwrap().is_empty());
        let request = {
            let replies = replies.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                replies
                    .request(&tx, json!({"subtype":"get_settings"}))
                    .await
            })
        };
        let _ = rx.recv().await.unwrap();
        replies.close();
        assert!(request.await.unwrap().is_err());
        assert!(replies
            .request(&tx, json!({"subtype":"get_settings"}))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn gateways_are_unavailable_and_codex_has_no_ultracode() {
        let (tx, _) = mpsc::channel(8);
        let replies = ClaudeReplies::new();
        init(&replies, true);
        let gateway = EffortControl::claude(replies, tx, true);
        assert!(gateway.status().await.reason.unwrap().contains("gateway"));
        let native = Arc::new(Mutex::new(pair("high", false)));
        let codex = EffortControl::codex(native.clone());
        codex.apply(&pair("low", false)).await.unwrap();
        assert_eq!(*native.lock().unwrap(), pair("low", false));
        assert!(codex.apply(&pair("xhigh", true)).await.is_err());
        codex.disconnected().await;
        assert!(!codex.status().await.available);
    }
}
