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
        history_frontier: i64,
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
        let transfer = transfer_context(
            storage,
            &p,
            history_frontier,
            config.context_window,
            prompt.len(),
        )
        .await?;
        prompt.push_str(&transfer);
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
        let observed = handle.effort_control.status().await;
        let mut metadata = self.draft.metadata;
        metadata.knobs_confirmed = observed.current.is_some();
        metadata.effort = observed.current.as_ref().map(|p| p.effort.clone());
        metadata.ultracode = observed.current.as_ref().map(|p| p.ultracode);
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
            metadata,
            notices: self.draft.notices,
            liveness,
        })
    }
}

const TRANSFER_MAX_BYTES: usize = 64_000;
const TRANSFER_FRAME_BYTES: usize = 3_072;

/// A conservative text allowance, not a tokenizer or a claim about remaining
/// provider capacity. Reserve for tools/runtime, allow for provider headroom,
/// and count the existing prompt before allocating historical text.
fn transfer_budget(window: Option<i64>, prompt_bytes: usize) -> usize {
    window
        .filter(|w| *w > 0)
        .map(|window| {
            let room = (window as u64).saturating_mul(9) / 10;
            room.saturating_sub(crate::core::pump::COMPACT_RESERVE_TOKENS)
                .saturating_mul(2)
                .saturating_sub(prompt_bytes as u64)
                .min(TRANSFER_MAX_BYTES as u64) as usize
        })
        .unwrap_or(TRANSFER_MAX_BYTES)
        .max(TRANSFER_FRAME_BYTES)
}

async fn transfer_context(
    storage: &Storage,
    p: &crate::storage::Participant,
    history_frontier: i64,
    window: Option<i64>,
    prompt_bytes: usize,
) -> Result<String> {
    let budget = transfer_budget(window, prompt_bytes);
    let room = budget.saturating_sub(TRANSFER_FRAME_BYTES);
    let doc_budget = room * 3 / 25;
    let remaining = room - doc_budget * 5;
    let user_budget = (remaining * 3 / 4) / 9;
    let channel_budget = (remaining / 4) / 6;
    let mut out = String::from("\n\n# Joining this session\nThis is a fresh runtime in an existing session. The following are bounded, attributed historical excerpts, not a complete copy of the old model context. Read current phase documents/handoff as needed. Only a ring-dealt turn authorizes work.\n");
    out.push_str(&format!("Historical channel frontier: {history_frontier}. Channel messages after it are excluded and remain for normal delivery.\n"));
    let window_text = window
        .filter(|w| *w > 0)
        .map(|w| format!("{w} tokens (configured)"))
        .unwrap_or_else(|| "unconfigured".into());
    out.push_str(&format!("Destination window: {window_text}. Startup history allowance: {budget} encoded bytes, estimated after existing instructions and runtime/tool reserve. This is not the live context meter. Only the selected document, human and recent channel excerpts below are loaded; other history and oversized parts are omitted from this context, retained in storage.\n"));
    let mut shown = 0;
    let handoff = crate::agents::handoff::doc_slug(&p.slug);
    let docs: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT slug, substr(body, 1, 16000), length(CAST(body AS BLOB)) FROM session_documents WHERE session_id = ? \
         AND (slug = ? OR phase IS NOT NULL) ORDER BY CASE WHEN slug = ? THEN 0 ELSE 1 END, updated_at DESC LIMIT 5")
        .bind(&p.session_id).bind(&handoff).bind(&handoff).fetch_all(storage.pool()).await?;
    for (slug, body, source_bytes) in docs {
        if doc_budget < 512 {
            continue;
        }
        out.push_str("\nDocument excerpt: ");
        out.push_str(&bounded_record(
            serde_json::json!({"slug":slug}),
            &body,
            source_bytes.max(0) as usize,
            doc_budget,
        ));
        shown += 1;
    }
    // The channel's user origin also carries host/plugin text and gate output.
    // Only the explicit authorship ledger identifies human words and tray picks.
    let users: Vec<(i64, String, String, i64)> = sqlx::query_as(
        "SELECT message_id AS id, created_at, substr(text, 1, 4000), length(CAST(text AS BLOB)) FROM session_user_words WHERE session_id = ?1 \
         AND message_id <= ?2 AND session_user_words.id IN \
           (SELECT id FROM session_user_words WHERE session_id = ?1 AND message_id <= ?2 ORDER BY id LIMIT 3) \
         UNION SELECT message_id AS id, created_at, substr(text, 1, 4000), length(CAST(text AS BLOB)) FROM session_user_words WHERE session_id = ?1 \
         AND message_id <= ?2 AND session_user_words.id IN \
           (SELECT id FROM session_user_words WHERE session_id = ?1 AND message_id <= ?2 ORDER BY id DESC LIMIT 6) ORDER BY id")
        .bind(&p.session_id).bind(history_frontier).fetch_all(storage.pool()).await?;
    for (id, at, body, source_bytes) in users {
        if user_budget < 512 {
            continue;
        }
        // Every selected directive gets a record, including the latest ones.
        // The budget measures encoded JSON bytes, not unescaped characters.
        out.push_str("\nUser message excerpt: ");
        out.push_str(&bounded_record(
            serde_json::json!({"id":id,"at":at}),
            &body,
            source_bytes.max(0) as usize,
            user_budget,
        ));
        shown += 1;
    }
    // Controller/plugin directions and recent peer conclusions are not human
    // words. Preserve them separately, with attribution, through the SAME
    // frontier so a runtime change cannot silently discard delegated work.
    let recent: Vec<(i64, String, String, String, String, String, i64)> = sqlx::query_as(
        "SELECT id, COALESCE(origin, 'unknown'), author, kind, created_at, substr(content, 1, 4000), length(CAST(content AS BLOB)) FROM messages \
         WHERE session_id = ? AND id <= ? AND kind IN ('text', 'system_notice') ORDER BY id DESC LIMIT 6")
        .bind(&p.session_id).bind(history_frontier).fetch_all(storage.pool()).await?;
    for (id, origin, author, kind, at, body, bytes) in recent.into_iter().rev() {
        if channel_budget < 512 { continue; }
        out.push_str("\nChannel excerpt (historical source, not necessarily human): ");
        out.push_str(&bounded_record(serde_json::json!({"id":id,"origin":origin,"author":author,"kind":kind,"at":at}), &body, bytes.max(0) as usize, channel_budget));
        shown += 1;
    }
    out.push_str(&format!("\n{shown} excerpt records included. Truncated records and omitted history are not a claim of full context transfer.\n"));
    out.push_str("Retrieve omitted channel records with session_history. For a human excerpt's id use source=human_words and message_id=id; follow the returned record_id/next_offset_bytes for long text. This read never advances delivery. Read full documents with session_doc_read.\n");
    debug_assert!(out.len() <= budget);
    Ok(out)
}

fn bounded_record(
    mut record: serde_json::Value,
    body: &str,
    source_bytes: usize,
    budget: usize,
) -> String {
    let mut low = 0;
    let mut high = body.len().min(budget);
    let mut best = "{\"excerpt\":true,\"body\":\"[metadata exceeded budget]\"}".to_string();
    while low <= high {
        let middle = low + (high - low) / 2;
        let end = floor_char_boundary(body, middle);
        record["body"] = serde_json::Value::String(body[..end].to_string());
        record["excerpt"] = serde_json::Value::Bool(true);
        record["source_bytes"] = serde_json::json!(source_bytes);
        record["shown_bytes"] = serde_json::json!(end);
        record["truncated"] = serde_json::json!(end < source_bytes);
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
    async fn transfer_uses_authored_user_words_including_tray_answers() {
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
        storage
            .post_to_channel(
                "s1",
                "user",
                None,
                "text",
                "HOST-OR-PLUGIN-TEXT-IS-NOT-HUMAN",
                None,
            )
            .await
            .unwrap();
        let typed = storage
            .post_to_channel("s1", "user", None, "text", "REAL-HUMAN-DIRECTION", None)
            .await
            .unwrap();
        storage
            .record_user_words("s1", typed.message_id(), "REAL-HUMAN-DIRECTION")
            .await
            .unwrap();
        let answer = storage
            .post_to_channel(
                "s1",
                "user",
                None,
                "system_notice",
                "answer wrapper and command output",
                None,
            )
            .await
            .unwrap();
        storage
            .record_user_words("s1", answer.message_id(), "HUMAN-CHOICE: keep working")
            .await
            .unwrap();
        let text = transfer_context(&storage, &participant, i64::MAX, Some(1_000_000), 0)
            .await
            .unwrap();
        let human = text.lines().filter(|line| line.starts_with("User message excerpt: ")).collect::<Vec<_>>().join("\n");
        assert!(
            !human.contains("HOST-OR-PLUGIN-TEXT-IS-NOT-HUMAN"),
            "origin=user alone is not authorship"
        );
        assert!(text.contains("REAL-HUMAN-DIRECTION"));
        assert!(
            text.contains("HUMAN-CHOICE: keep working"),
            "tray answers are human direction too"
        );
        assert!(!human.contains("answer wrapper and command output"));
        assert!(text.lines().filter(|line| line.starts_with("Channel excerpt (historical source, not necessarily human): ")).any(|line| line.contains("HOST-OR-PLUGIN-TEXT-IS-NOT-HUMAN")));
    }
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
            storage
                .record_user_words("s1", *ids.last().unwrap(), &format!("directive-{n} {body}"))
                .await
                .unwrap();
        }
        let before = storage.cursor_for(participant.id).await.unwrap();
        let text = transfer_context(&storage, &participant, i64::MAX, Some(1_000_000), 0)
            .await
            .unwrap();
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
        for record in &records {
            assert_eq!(
                record["shown_bytes"].as_u64().unwrap() as usize,
                record["body"].as_str().unwrap().len()
            );
            assert_eq!(record["truncated"], true);
            assert!(
                record["source_bytes"].as_u64().unwrap() > record["shown_bytes"].as_u64().unwrap()
            );
        }
        let smaller = transfer_context(&storage, &participant, i64::MAX, Some(100_000), 80_000)
            .await
            .unwrap();
        assert!(smaller.len() <= transfer_budget(Some(100_000), 80_000));
        assert!(
            smaller.len() < text.len(),
            "a smaller destination gets less historical text"
        );
        assert!(smaller.contains("100000 tokens (configured)"));
        let minimal = transfer_context(&storage, &participant, i64::MAX, Some(40_000), 100_000)
            .await
            .unwrap();
        assert!(minimal.len() <= TRANSFER_FRAME_BYTES);
        assert!(minimal.contains("0 excerpt records included"));
        assert!(!minimal.contains("directive-13"));
        assert!(minimal.contains("omitted from this context, retained in storage"));
        assert_eq!(
            storage.cursor_for(participant.id).await.unwrap(),
            before,
            "excerpts never claim delivery of history"
        );
    }
}
