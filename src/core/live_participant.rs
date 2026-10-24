//! A single participant's candidate, sharing the live session's ring/tracker.
use super::*;
use crate::core::participant_runtime::SpawnServices;
use crate::core::sequencer::SequencerCommand;
use tokio::sync::mpsc;

#[derive(Clone)]
pub(crate) struct LiveContext {
    pub activity: Arc<crate::core::ActivityTracker>,
    pub ipav: Arc<Mutex<IpavState>>,
    pub atomic: Arc<std::sync::atomic::AtomicBool>,
    pub booting: Arc<std::sync::atomic::AtomicBool>,
    pub orienting: crate::core::sequencer::OrientingSet,
    pub watchdog: crate::core::watchdog::LiveWatchdogRoster,
    pub working_dir: Option<PathBuf>,
    pub ring: mpsc::Sender<SequencerCommand>,
}

impl LiveContext {
    pub fn capture(handle: &SessionHandle, ring: mpsc::Sender<SequencerCommand>) -> Self {
        Self {
            activity: handle.activity.clone(),
            ipav: handle.ipav.clone(),
            atomic: handle.in_atomic_tool.clone(),
            booting: handle.booting.clone(),
            orienting: handle.orienting.clone(),
            watchdog: handle.watchdog_roster.clone(),
            working_dir: handle.working_repo_path.clone(),
            ring,
        }
    }
    pub fn matches(&self, handle: &SessionHandle) -> bool {
        Arc::ptr_eq(&self.activity, &handle.activity)
    }
    pub async fn hold_watchdog(&self) {
        let scope = {
            let mut roster = self.watchdog.lock().unwrap_or_else(|p| p.into_inner());
            roster.changing = true;
            roster.scope.clone()
        };
        scope.retire().await;
    }
}

pub(crate) struct CandidateDraft {
    p: crate::storage::Participant,
    draft: ParticipantSpawnDraft,
    temp: TempDir,
}

pub(crate) struct LiveCandidate {
    pub agent: SessionAgent,
    pub events: mpsc::Receiver<crate::agents::AgentEvent>,
    pub pump: PumpConfig,
    pub metadata: crate::storage::ParticipantSpawnMetadata,
    pub notices: Vec<String>,
    pub liveness: Arc<crate::core::watchdog::AgentLiveness>,
}

impl CandidateDraft {
    pub async fn prepare(
        storage: &Storage,
        paths: &Paths,
        bridge: &Arc<SignalingBridge>,
        addr: SocketAddr,
        context: &LiveContext,
        p: crate::storage::Participant,
        config: AgentConfig,
        role: Option<crate::storage::Role>,
        services: &SpawnServices,
    ) -> Result<Self> {
        let session = storage
            .get_session(&p.session_id)
            .await?
            .context("session disappeared")?;
        let (project, _) = resolve_session_project(
            storage,
            session.base_repo_path.as_deref(),
            context.working_dir.as_deref(),
        )
        .await;
        let project_root = match project.as_deref() {
            Some(project) => storage
                .cl_path_for_project(&paths.data_dir, project)
                .await
                .ok(),
            None => None,
        };
        let index = match project.as_deref() {
            Some(project) => storage
                .cl_index_search_agent(Some(project), None)
                .await
                .ok(),
            None => None,
        };
        let role = match role {
            Some(role) => Some(role),
            None => match p.role_id {
                Some(id) => storage.role_by_id(id).await?,
                None => None,
            },
        };
        let roster = storage.participants_for_session(&p.session_id).await?;
        let facts = resolve_roster_facts(storage, &roster, &p).await;
        let mut prompt = read_system_prompt(
            paths,
            project.as_deref(),
            project_root.as_deref(),
            index.as_deref(),
            role.as_ref().and_then(|r| r.description_prompt.as_deref()),
            facts.as_ref(),
        )?;
        // Bounded host records, explicitly excerpts. Existing readers keep
        // their cursors; no tentative preparation acknowledges unread rows.
        prompt.push_str(&transfer_context(storage, &p).await?);
        let overrides = crate::claude_config::resolve_agent_overrides(
            &crate::claude_config::load_overrides(&paths.data_dir),
            role.as_ref().map(|r| r.slug.as_str()),
        );
        let temp = TempDir::new().context("creating candidate prompt directory")?;
        let draft = participant_spawn_draft(
            storage,
            &p,
            config,
            paths,
            &project,
            prompt,
            addr,
            temp.path(),
            context.working_dir.clone(),
            bridge,
            &*services.auth,
            &*services.codex,
            overrides,
        )
        .await?;
        Ok(Self { p, draft, temp })
    }

    pub fn token(&self) -> &str {
        &self.draft.mcp_token
    }
    pub fn slug(&self) -> &str {
        &self.p.slug
    }

    pub async fn spawn(
        self,
        context: &LiveContext,
        bridge: &Arc<SignalingBridge>,
        services: &SpawnServices,
    ) -> Result<LiveCandidate> {
        let config = self.draft.config.config.clone();
        let account_dir = self.draft.config.account_dir.clone().unwrap_or_default();
        let account_org = self
            .draft
            .config
            .account_org
            .clone()
            .or(self.p.account_org_at_spawn.clone());
        let account_email = self.draft.config.account_email.clone();
        let prompt_path = self.draft.config.system_prompt_path.clone();
        let mut handle = (services.spawn)(self.draft.config).await?;
        let events = std::mem::replace(&mut handle.event_rx, mpsc::channel(1).1);
        let epoch = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let liveness = crate::core::watchdog::AgentLiveness::new();
        let capabilities = participant_capabilities(&self.p);
        let pump = PumpConfig {
            participant_id: Some(self.p.id),
            bridge: Some(bridge.clone()),
            activity: Some(context.activity.clone()),
            in_atomic_tool: Some(context.atomic.clone()),
            liveness: Some(liveness.clone()),
            sequencer_tx: Some(context.ring.clone()),
            turn_epoch: Some(epoch.clone()),
            self_nudges: capabilities.grants(crate::agents::Capability::EditFiles),
            edits_files: capabilities.grants(crate::agents::Capability::EditFiles),
            configured_context_window: config.context_window.and_then(|v| u64::try_from(v).ok()),
            bills_subscription: bills_a_subscription(&config),
            account_dir: Some(account_dir),
            account_org,
            account_email,
            model_name: config.model_name.clone(),
            booting: Some(context.booting.clone()),
            orienting: Some(context.orienting.clone()),
            ..PumpConfig::new(
                self.p.session_id.clone(),
                self.p.slug.clone(),
                handle.interrupted_epoch(),
                handle.incarnation(),
            )
        };
        Ok(LiveCandidate {
            agent: SessionAgent {
                participant_id: Some(self.p.id),
                slug: self.p.slug,
                turn_position: self.p.turn_position,
                capabilities,
                system_prompt_path: prompt_path,
                handle,
                turn_epoch: Some(epoch),
                _spawn_temp: Some(self.temp),
                mcp_token: Some(self.draft.mcp_token),
                config_snapshot: Some(config),
            },
            events,
            pump,
            metadata: self.draft.metadata,
            notices: self.draft.notices,
            liveness,
        })
    }
}

async fn transfer_context(storage: &Storage, p: &crate::storage::Participant) -> Result<String> {
    let mut out = String::from("\n\n# Joining this session\nThis is a fresh runtime in an existing session. The following are bounded, attributed historical excerpts, not a complete copy of the old model context. Read current phase documents/handoff as needed. Only a ring-dealt turn authorizes work.\n");
    let handoff = crate::agents::handoff::doc_slug(&p.slug);
    let docs: Vec<(String, String)> = sqlx::query_as(
        "SELECT slug, substr(body, 1, 16000) FROM session_documents WHERE session_id = ? \
         AND (slug = ? OR phase IS NOT NULL) ORDER BY CASE WHEN slug = ? THEN 0 ELSE 1 END, updated_at DESC LIMIT 5")
        .bind(&p.session_id).bind(&handoff).bind(&handoff).fetch_all(storage.pool()).await?;
    for (slug, body) in docs {
        out.push_str("\nDocument excerpt: ");
        out.push_str(&bounded_record(
            serde_json::json!({"slug":slug}),
            &body,
            7_500,
        ));
    }
    let users: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT id, created_at, substr(content, 1, 4000) FROM messages WHERE session_id = ? AND origin = 'user' AND kind = 'text' \
         AND id IN (SELECT id FROM messages WHERE session_id = ? AND origin = 'user' AND kind = 'text' ORDER BY id LIMIT 3) \
         UNION SELECT id, created_at, substr(content, 1, 4000) FROM messages WHERE session_id = ? AND origin = 'user' AND kind = 'text' \
         AND id IN (SELECT id FROM messages WHERE session_id = ? AND origin = 'user' AND kind = 'text' ORDER BY id DESC LIMIT 6) ORDER BY id")
        .bind(&p.session_id).bind(&p.session_id).bind(&p.session_id).bind(&p.session_id).fetch_all(storage.pool()).await?;
    for (id, at, body) in users {
        // Every selected directive gets a record, including the latest ones.
        // The budget measures encoded JSON bytes, not unescaped characters.
        out.push_str("\nUser message excerpt: ");
        out.push_str(&bounded_record(
            serde_json::json!({"id":id,"at":at}),
            &body,
            2_800,
        ));
    }
    debug_assert!(out.len() <= 64_000);
    Ok(out)
}

fn bounded_record(mut record: serde_json::Value, body: &str, budget: usize) -> String {
    let mut low = 0;
    let mut high = body.len().min(budget);
    let mut best = "{\"excerpt\":true,\"body\":\"[metadata exceeded budget]\"}".to_string();
    while low <= high {
        let middle = low + (high - low) / 2;
        let end = floor_char_boundary(body, middle);
        record["body"] = serde_json::Value::String(body[..end].to_string());
        record["excerpt"] = serde_json::Value::Bool(true);
        let encoded = record.to_string();
        if encoded.len() <= budget {
            best = encoded;
            low = middle + 1;
        } else if middle == 0 {
            break;
        } else {
            high = middle - 1;
        }
    }
    best
}

pub(crate) fn refresh_runtime_roster(
    handle: &mut SessionHandle,
    storage: &Storage,
    bridge: &Arc<SignalingBridge>,
    roster: &[crate::storage::Participant],
) {
    let live = resolve_spawn_roster(bridge, &handle.id, roster);
    handle.participants.sort_by_key(|p| p.turn_position);
    handle
        .activity
        .replace_roster(handle.participants.iter().map(|p| p.slug.clone()).collect());
    handle
        .liveness
        .retain(|(slug, _)| handle.participants.iter().any(|a| &a.slug == slug));
    let target = live
        .iter()
        .find(|p| participant_capabilities(p).grants(crate::agents::Capability::EditFiles))
        .or(live.first());
    *handle
        .watchdog_roster
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = crate::core::watchdog::WatchdogRoster {
        scope: crate::agents::incarnation::Incarnation::new(),
        agents: handle.liveness.clone(),
        hands_slug: target.map(|p| p.slug.clone()),
        changing: false,
        idle_watch: target.map(|p| crate::core::watchdog::IdleWatch {
            storage: storage.clone(),
            hands_participant_id: Some(p.id),
            ipav: handle.ipav.clone(),
            user_broadcasts: handle.user_broadcasts.clone(),
        }),
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn transfer_caps_encoded_bytes_and_keeps_every_selected_user_record() {
        let storage = Storage::memory().await.unwrap();
        storage
            .create_session("s1", "transfer", None)
            .await
            .unwrap();
        storage.ensure_session_roster("s1", 1).await.unwrap();
        let participant = storage
            .participants_for_session("s1")
            .await
            .unwrap()
            .remove(0);
        let body = "\u{0001}\t\n🌱\"\\".repeat(10_000);
        for phase in ["investigate", "plan", "apply", "verify"] {
            storage
                .upsert_session_document("s1", phase, &body, Some(phase))
                .await
                .unwrap();
        }
        storage
            .upsert_session_document(
                "s1",
                &crate::agents::handoff::doc_slug(&participant.slug),
                &body,
                None,
            )
            .await
            .unwrap();
        let mut ids = vec![];
        for n in 0..14 {
            ids.push(
                storage
                    .post_to_channel(
                        "s1",
                        "user",
                        None,
                        "text",
                        format!("directive-{n} {body}"),
                        None,
                    )
                    .await
                    .unwrap()
                    .message_id(),
            );
        }
        let before = storage.cursor_for(participant.id).await.unwrap();
        let text = transfer_context(&storage, &participant).await.unwrap();
        assert!(text.len() <= 64_000, "encoded size {}", text.len());
        let records: Vec<serde_json::Value> = text
            .lines()
            .filter_map(|line| line.strip_prefix("User message excerpt: "))
            .map(|body| serde_json::from_str(body).unwrap())
            .collect();
        assert_eq!(records.len(), 9);
        let selected: Vec<i64> = records.iter().map(|v| v["id"].as_i64().unwrap()).collect();
        assert_eq!(
            selected,
            ids[..3]
                .iter()
                .chain(&ids[8..])
                .copied()
                .collect::<Vec<_>>()
        );
        assert!(records.last().unwrap()["body"]
            .as_str()
            .unwrap()
            .starts_with("directive-13"));
        assert_eq!(
            storage.cursor_for(participant.id).await.unwrap(),
            before,
            "excerpts never claim delivery of history"
        );
    }
}
