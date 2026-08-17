use bot_hq::storage::{AgentConfig, MessageKind, Storage};

#[tokio::test]
async fn migration_runs_on_empty_db() {
    let s = Storage::memory().await.unwrap();
    // `agent_configs` seeds NOTHING now. It used to arrive with `brian` and
    // `rain` rows (and `emma` until 0017) behind a CHECK that allowed only
    // those three names — so the table could not hold a key any live session
    // produces. 0060 rebuilt it without the CHECK and dropped the rows.
    let cfgs = s.list_agent_configs().await.unwrap();
    assert!(
        cfgs.is_empty(),
        "expected an empty agent_configs, found {:?}",
        cfgs.iter().map(|c| c.agent_name.as_str()).collect::<Vec<_>>()
    );
}

/// **Migration 0066 — the messages indexes earn their writes** (round 8).
/// Three measured-dead-or-once-per-session indexes are gone, one seek index
/// exists, and the two the measurement KEPT are still there. Kill-tested:
/// comment out a DROP in 0066 → red.
#[tokio::test]
async fn migration_0066_drops_the_unearned_indexes_and_adds_the_seek() {
    let s = Storage::memory().await.unwrap();
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'index' \
         AND tbl_name IN ('messages', 'session_documents') ORDER BY name",
    )
    .fetch_all(s.pool())
    .await
    .unwrap();
    for gone in [
        "idx_messages_session_participant_time",
        "idx_messages_session_author_time",
        "session_documents_session_idx",
    ] {
        assert!(!names.iter().any(|n| n == gone), "{gone} should be dropped by 0066: {names:?}");
    }
    for kept in [
        "idx_messages_session_time",
        "idx_messages_session_id",
        "idx_messages_participant_kind_id",
        "session_documents_phase_idx",
    ] {
        assert!(names.iter().any(|n| n == kept), "{kept} must exist: {names:?}");
    }
    // And the spin-detection read seeks on the new index.
    // The production string, not a copy of it (round 9).
    let sql = format!("EXPLAIN QUERY PLAN {}", bot_hq::storage::participant_text_since_sql());
    let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(&sql)
    .bind(1_i64)
    .bind(0_i64)
    .bind(64_i64)
    .fetch_all(s.pool())
    .await
    .unwrap();
    let detail = plan.iter().map(|r| r.3.as_str()).collect::<Vec<_>>().join(" | ");
    assert!(
        detail.contains("idx_messages_participant_kind_id"),
        "participant_text_since must seek on the new index, got: {detail}"
    );
}

#[tokio::test]
async fn pending_tray_open_sessions_excludes_closed() {
    use bot_hq::storage::QuestionKind;
    let s = Storage::memory().await.unwrap();
    s.create_session("open-s", "open", Some("/tmp/r"))
        .await
        .unwrap();
    s.create_session("closed-s", "closed", Some("/tmp/r"))
        .await
        .unwrap();

    let opts = vec!["Approve".to_string(), "Reject".to_string()];
    // One pending each: an open session and a (soon-)closed session.
    for (sid, cid) in [("open-s", "c-open"), ("closed-s", "c-closed")] {
        s.insert_tray_entry(
            sid,
            cid,
            "brian",
            QuestionKind::Choice,
            "go?",
            Some(&opts),
            None,
            None,
        )
        .await
        .unwrap();
    }
    s.close_session("closed-s", false).await.unwrap();

    let pending = s.pending_tray_open_sessions().await.unwrap();
    let ids: Vec<&str> = pending.iter().map(|p| p.choice_id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["c-open"],
        "only the open session's pending should surface; closed excluded"
    );
}

#[tokio::test]
async fn withdraw_pending_tray_for_session_scoped_and_only_pending() {
    use bot_hq::storage::QuestionKind;
    let s = Storage::memory().await.unwrap();
    s.create_session("a", "a", None).await.unwrap();
    s.create_session("b", "b", None).await.unwrap();
    let opts = vec!["Yes".to_string(), "No".to_string()];
    for (sid, cid) in [("a", "ca1"), ("a", "ca2"), ("b", "cb1")] {
        s.insert_tray_entry(
            sid,
            cid,
            "brian",
            QuestionKind::Choice,
            "q",
            Some(&opts),
            None,
            None,
        )
        .await
        .unwrap();
    }
    // Answer one of a's so the withdraw only touches the still-pending row.
    s.answer_tray_entry("ca1", "Yes").await.unwrap();

    let n = s.withdraw_pending_tray_for_session("a").await.unwrap();
    assert_eq!(n, 1, "only a's remaining pending row is withdrawn");

    let a = s.tray_entries_for_session("a").await.unwrap();
    let ca1 = a.iter().find(|r| r.choice_id == "ca1").unwrap();
    let ca2 = a.iter().find(|r| r.choice_id == "ca2").unwrap();
    assert_eq!(ca1.status, "answered", "already-answered row untouched");
    assert_eq!(ca2.status, "withdrawn");
    // Session b is untouched.
    let b = s.tray_entries_for_session("b").await.unwrap();
    assert_eq!(b[0].status, "pending");
}

#[tokio::test]
async fn message_round_trip() {
    let s = Storage::memory().await.unwrap();
    s.create_session("sess1", "test session", Some("/tmp/repo"))
        .await
        .unwrap();
    let id1 = s
        .insert_user_message("sess1", MessageKind::Text, "hello")
        .await
        .unwrap()
        .message_id();
    let id2 = s
        .post_to_channel("sess1", "participant", Some("hands"), MessageKind::Text.as_str(), "world", None)
        .await
        .unwrap()
        .message_id();
    assert!(id2 > id1);
    let msgs = s.messages_for_session("sess1", None).await.unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].content, "hello");
    assert_eq!(msgs[1].content, "world");
    // Author strings, not an enum: `Author` and its `author_typed` accessor were
    // deleted in the D10 retirement. The enum could only ever name `user`,
    // `brian` and `rain`, so it could not describe a single participant this
    // app now creates — `author_typed` returned `None` for every live slug and
    // had no callers outside this line.
    assert_eq!(msgs[0].author, "user");
    assert_eq!(msgs[1].author, "hands");
}

#[tokio::test]
async fn messages_since_id_filter() {
    let s = Storage::memory().await.unwrap();
    s.create_session("sess1", "test", None).await.unwrap();
    let id1 = s
        .insert_user_message("sess1", MessageKind::Text, "a")
        .await
        .unwrap()
        .message_id();
    s.post_to_channel("sess1", "participant", Some("hands"), MessageKind::Text.as_str(), "b", None)
        .await
        .unwrap();
    s.post_to_channel("sess1", "participant", Some("eyes"), MessageKind::Text.as_str(), "c", None)
        .await
        .unwrap();
    let after = s.messages_for_session("sess1", Some(id1)).await.unwrap();
    assert_eq!(after.len(), 2);
    assert_eq!(after[0].content, "b");
    assert_eq!(after[1].content, "c");
}

#[tokio::test]
async fn close_session_marks_closed() {
    let s = Storage::memory().await.unwrap();
    s.create_session("sess1", "test", None).await.unwrap();
    s.close_session("sess1", true).await.unwrap();
    let sess = s.get_session("sess1").await.unwrap().unwrap();
    assert!(sess.closed_at.is_some());
    assert_eq!(sess.archived, 1);
}

#[tokio::test]
async fn active_sessions_excludes_closed() {
    let s = Storage::memory().await.unwrap();
    s.create_session("active1", "a", None).await.unwrap();
    s.create_session("closed1", "c", None).await.unwrap();
    s.close_session("closed1", true).await.unwrap();
    let active = s.list_active_sessions().await.unwrap();
    let ids: Vec<_> = active.iter().map(|s| s.id.as_str()).collect();
    assert!(ids.contains(&"active1"));
    assert!(!ids.contains(&"closed1"));
}

#[tokio::test]
async fn upsert_agent_config_overwrites() {
    let s = Storage::memory().await.unwrap();
    // Insert-then-overwrite on a ROLE slug, which only became a legal key when
    // 0060 dropped the legacy CHECK.
    let cfg = AgentConfig {
        agent_name: "hands".into(),
        provider: "anthropic".into(),
        model_name: "seed".into(),
        base_url: None,
        auth_token: None,
        updated_at: String::new(),
        context_window: None,
    };
    s.upsert_agent_config(&cfg).await.unwrap();
    let mut cfg = s.get_agent_config("hands").await.unwrap().unwrap();
    cfg.model_name = "claude-haiku-4-5".to_string();
    cfg.auth_token = Some("sk-test-123".to_string());
    s.upsert_agent_config(&cfg).await.unwrap();
    let reloaded = s.get_agent_config("hands").await.unwrap().unwrap();
    assert_eq!(reloaded.model_name, "claude-haiku-4-5");
    assert_eq!(reloaded.auth_token.as_deref(), Some("sk-test-123"));
}

#[tokio::test]
async fn upsert_agent_config_inserts_new_via_constructor() {
    // No row is seeded any more and any key is legal, so the constructor path
    // inserts a fresh ROLE slug rather than overwriting a seeded legacy name.
    let s = Storage::memory().await.unwrap();
    let cfg = AgentConfig {
        agent_name: "eyes".into(),
        provider: "deepseek".into(),
        model_name: "deepseek-coder".into(),
        base_url: Some("https://api.deepseek.com".into()),
        auth_token: Some("ds-token".into()),
        updated_at: String::new(), // ignored on insert/upsert
        context_window: None,
    };
    s.upsert_agent_config(&cfg).await.unwrap();
    let got = s.get_agent_config("eyes").await.unwrap().unwrap();
    assert_eq!(got.provider, "deepseek");
    assert_eq!(got.model_name, "deepseek-coder");
    assert_eq!(got.base_url.as_deref(), Some("https://api.deepseek.com"));
}

/// The projection lost a column but the TABLE did not (rc3 D9 leaves
/// `agent_configs.native` in place, unread), so the surviving columns must still
/// bind to their own slots — dropping one name from the column list and leaving
/// the `bind()` order alone shifts every value one place, silently.
#[tokio::test]
async fn agent_config_round_trips_every_column_it_still_projects() {
    let s = Storage::memory().await.unwrap();
    let cfg = AgentConfig {
        agent_name: "rain".into(),
        provider: "deepseek".into(),
        model_name: "deepseek-v4-pro".into(),
        base_url: Some("https://api.deepseek.com/anthropic".into()),
        auth_token: Some("ds-token".into()),
        updated_at: String::new(),
        context_window: Some(1_000_000),
    };
    s.upsert_agent_config(&cfg).await.unwrap();

    let got = s.get_agent_config("rain").await.unwrap().unwrap();
    assert_eq!(got.provider, "deepseek");
    assert_eq!(got.model_name, "deepseek-v4-pro");
    assert_eq!(
        got.base_url.as_deref(),
        Some("https://api.deepseek.com/anthropic")
    );
    assert_eq!(got.auth_token.as_deref(), Some("ds-token"));
    assert_eq!(got.context_window, Some(1_000_000));

    // And a written value must clear again — a field that only ever latches on
    // is its own bug.
    let back = AgentConfig {
        context_window: None,
        ..got
    };
    s.upsert_agent_config(&back).await.unwrap();
    let got = s.get_agent_config("rain").await.unwrap().unwrap();
    assert_eq!(got.context_window, None);
}

/// Same guard for `models`, and it is the one that matters most: `MODEL_COLUMNS`
/// and the `bind()` list are two hand-maintained sequences that must stay in
/// step, and rc3 D9 shortened both by one.
#[tokio::test]
async fn model_round_trips_every_column_it_still_projects() {
    let s = Storage::memory().await.unwrap();
    let m = bot_hq::storage::Model {
        id: "m-gateway".into(),
        display_name: "DeepSeek V4 Pro".into(),
        provider: "deepseek".into(),
        model_name: "deepseek-v4-pro".into(),
        base_url: Some("https://api.deepseek.com/anthropic".into()),
        auth_token: Some("ds-token".into()),
        created_at: String::new(),
        updated_at: String::new(),
        context_window: Some(1_000_000),
    };
    s.upsert_model(&m).await.unwrap();

    let got = s.get_model("m-gateway").await.unwrap().unwrap();
    assert_eq!(got.context_window, Some(1_000_000));
    // Every other column must land in its own slot, not a neighbour's.
    assert_eq!(got.display_name, "DeepSeek V4 Pro");
    assert_eq!(got.provider, "deepseek");
    assert_eq!(got.model_name, "deepseek-v4-pro");
    assert_eq!(
        got.base_url.as_deref(),
        Some("https://api.deepseek.com/anthropic")
    );
    assert_eq!(got.auth_token.as_deref(), Some("ds-token"));

    let off = bot_hq::storage::Model {
        context_window: None,
        ..got
    };
    s.upsert_model(&off).await.unwrap();
    let got = s.get_model("m-gateway").await.unwrap().unwrap();
    assert_eq!(got.context_window, None);
}



/// **The slot writer that survived round-3 F7, round-tripped.**
///
/// F7 deleted `set_session_spawn_models` and its round-trip test — correct, it
/// had no reader. But that test was the last thing exercising a WRITE to
/// `slot0_model_at_spawn` / `slot1_model_at_spawn`, and the live replacement
/// (`set_session_spawn_model_slot`, called once from `spawn_session_handle`)
/// had no test at all: six references in the tree, all definition, call site
/// and a `warn!` string. EYES filed it against the uncommitted diff (`579f1324`)
/// — the deletion was correct and coverage-negative at the same time.
///
/// Worth pinning rather than trusting, because every property here is silent
/// when wrong: both call sites swallow the error into `warn!`, the columns are
/// written positionally so swapping the arms is invisible to the type system,
/// and the result is only observable as the chat header's frozen model name.
/// The `_ =>` arm is the same shape — slot 2 exists (the backend caps at 8
/// participants, this schema holds 2) and drops its write in silence, which is
/// a real limit worth stating in a test rather than discovering later.
#[tokio::test]
async fn spawn_model_slots_round_trip_and_slot_two_is_a_silent_no_op() {
    let s = Storage::memory().await.unwrap();
    s.create_session("sess-slots", "t", None).await.unwrap();

    // Slot -> column, positionally. Asserted apart so a swapped pair fails.
    s.set_session_spawn_model_slot("sess-slots", 0, "claude-opus-5").await.unwrap();
    s.set_session_spawn_model_slot("sess-slots", 1, "claude-sonnet-5").await.unwrap();
    let got = s.get_session("sess-slots").await.unwrap().unwrap();
    assert_eq!(got.slot0_model_at_spawn.as_deref(), Some("claude-opus-5"), "slot 0 is the first column");
    assert_eq!(got.slot1_model_at_spawn.as_deref(), Some("claude-sonnet-5"), "slot 1 is the second");

    // The solo path: spawn clears slot 1 when only one participant spawned.
    s.clear_session_spawn_model_slot("sess-slots", 1).await.unwrap();
    let got = s.get_session("sess-slots").await.unwrap().unwrap();
    assert_eq!(got.slot1_model_at_spawn, None, "clearing writes SQL NULL");
    assert_eq!(
        got.slot0_model_at_spawn.as_deref(),
        Some("claude-opus-5"),
        "clearing one slot must not touch the other"
    );

    // Slot 2+ has no column: Ok(()) and nothing written. A third participant's
    // spawn model is NOT recorded anywhere — the stated limit of a 2-column
    // schema under an N-participant roster (round-3 F7).
    s.set_session_spawn_model_slot("sess-slots", 2, "some-model").await.unwrap();
    let after = s.get_session("sess-slots").await.unwrap().unwrap();
    assert_eq!(after.slot0_model_at_spawn, got.slot0_model_at_spawn, "slot 2 wrote nothing");
    assert_eq!(after.slot1_model_at_spawn, None, "slot 2 wrote nothing");
}

/// **Every `SESSION_COLUMNS` query is EXECUTED here, not just compiled.**
///
/// `SESSION_COLUMNS`' own comment states the hazard: *"a missing column fails
/// `query_as` at runtime, not compile time."* Migration 0060 dropped nine
/// columns from `sessions`, renamed two, and replaced `rain_enabled` with a
/// correlated subquery — so a projection that disagreed with the `Session`
/// struct would build perfectly and fail only when a query ran. Nothing else in
/// the suite forces all three query shapes.
///
/// Required as a definition-of-done by the reviewer before 0060 shipped, and it
/// is the right shape for the risk: the three sites share one const, so an edit
/// to it can only be caught by running each of them.
#[tokio::test]
async fn every_session_projection_executes_after_0060() {
    let s = Storage::memory().await.unwrap();
    s.create_session("s-proj", "t", Some("/tmp/r")).await.unwrap();

    // 1. get_session — the single-row path.
    let one = s.get_session("s-proj").await.unwrap().expect("row reads back");
    assert_eq!(one.id, "s-proj");
    assert!(!one.multi_participant, "no roster seeded yet, so not multi");
    assert_eq!(one.slot0_model_at_spawn, None);
    assert_eq!(one.slot1_model_at_spawn, None);

    // 2. list_active_sessions — the multi-row path.
    let active = s.list_active_sessions().await.unwrap();
    assert!(active.iter().any(|x| x.id == "s-proj"), "listed");

    // 3. The flattened preview path, whose prefix is the same const.
    let preview = s.list_active_sessions_with_preview().await.unwrap();
    assert!(preview.iter().any(|x| x.session.id == "s-proj"), "previewed");

    // The derived column tracks the ROSTER, which is the whole reason
    // `rain_enabled` was dropped rather than renamed: seed participants and the
    // same query answers differently, with nothing to write and nothing to
    // fall out of sync.
    s.ensure_session_roster("s-proj", bot_hq::storage::MAX_SESSION_PARTICIPANTS)
        .await
        .unwrap();
    let after = s.get_session("s-proj").await.unwrap().unwrap();
    assert!(
        after.multi_participant,
        "a seeded roster of 2+ must read as multi-participant"
    );
}
