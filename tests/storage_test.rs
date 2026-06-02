use bot_hq::storage::{AgentConfig, Author, MessageKind, Storage};

#[tokio::test]
async fn migration_runs_on_empty_db() {
    let s = Storage::memory().await.unwrap();
    let cfgs = s.list_agent_configs().await.unwrap();
    let names: Vec<_> = cfgs.iter().map(|c| c.agent_name.as_str()).collect();
    assert!(names.contains(&"emma"));
    assert!(names.contains(&"brian"));
    assert!(names.contains(&"rain"));
}

#[tokio::test]
async fn emma_singleton_seeded() {
    let s = Storage::memory().await.unwrap();
    let emma = s.get_session("emma").await.unwrap();
    assert!(emma.is_some(), "emma session must be seeded by migration");
    let emma = emma.unwrap();
    assert_eq!(emma.title, "Emma");
    assert!(emma.working_repo_path.is_none());
}

#[tokio::test]
async fn emma_seed_is_idempotent_across_open() {
    let s1 = Storage::memory().await.unwrap();
    let s2 = Storage::memory().await.unwrap();
    assert!(s1.get_session("emma").await.unwrap().is_some());
    assert!(s2.get_session("emma").await.unwrap().is_some());
}

#[tokio::test]
async fn pending_tray_open_sessions_excludes_closed_and_emma() {
    use bot_hq::storage::QuestionKind;
    let s = Storage::memory().await.unwrap();
    s.create_session("open-s", "open", Some("/tmp/r")).await.unwrap();
    s.create_session("closed-s", "closed", Some("/tmp/r")).await.unwrap();

    let opts = vec!["Approve".to_string(), "Reject".to_string()];
    // One pending each: an open session, a (soon-)closed session, and emma.
    for (sid, cid) in [
        ("open-s", "c-open"),
        ("closed-s", "c-closed"),
        ("emma", "c-emma"),
    ] {
        s.insert_question(sid, cid, "brian", QuestionKind::Choice, "go?", Some(&opts), None, None)
            .await
            .unwrap();
    }
    s.close_session("closed-s", false).await.unwrap();

    let pending = s.pending_tray_open_sessions().await.unwrap();
    let ids: Vec<&str> = pending.iter().map(|p| p.choice_id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["c-open"],
        "only the open, non-emma session's pending should surface; closed + emma excluded"
    );
}

#[tokio::test]
async fn message_round_trip() {
    let s = Storage::memory().await.unwrap();
    s.create_session("sess1", "test session", Some("/tmp/repo"))
        .await
        .unwrap();
    let id1 = s
        .insert_message("sess1", Author::User, MessageKind::Text, "hello")
        .await
        .unwrap();
    let id2 = s
        .insert_message("sess1", Author::Brian, MessageKind::Text, "world")
        .await
        .unwrap();
    assert!(id2 > id1);
    let msgs = s.messages_for_session("sess1", None).await.unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].content, "hello");
    assert_eq!(msgs[1].content, "world");
    assert_eq!(msgs[0].author_typed(), Some(Author::User));
    assert_eq!(msgs[1].author_typed(), Some(Author::Brian));
}

#[tokio::test]
async fn messages_since_id_filter() {
    let s = Storage::memory().await.unwrap();
    s.create_session("sess1", "test", None).await.unwrap();
    let id1 = s
        .insert_message("sess1", Author::User, MessageKind::Text, "a")
        .await
        .unwrap();
    s.insert_message("sess1", Author::Brian, MessageKind::Text, "b")
        .await
        .unwrap();
    s.insert_message("sess1", Author::Rain, MessageKind::Text, "c")
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
async fn active_sessions_excludes_closed_and_emma() {
    let s = Storage::memory().await.unwrap();
    s.create_session("active1", "a", None).await.unwrap();
    s.create_session("closed1", "c", None).await.unwrap();
    s.close_session("closed1", true).await.unwrap();
    let active = s.list_active_sessions().await.unwrap();
    let ids: Vec<_> = active.iter().map(|s| s.id.as_str()).collect();
    // Emma is a chat-overlay singleton — explicitly filtered out so she
    // doesn't surface as a phantom Dashboard tile alongside duo sessions.
    assert!(!ids.contains(&"emma"));
    assert!(ids.contains(&"active1"));
    assert!(!ids.contains(&"closed1"));
}

#[tokio::test]
async fn upsert_agent_config_overwrites() {
    let s = Storage::memory().await.unwrap();
    let mut cfg = s.get_agent_config("brian").await.unwrap().unwrap();
    cfg.model_name = "claude-haiku-4-5".to_string();
    cfg.auth_token = Some("sk-test-123".to_string());
    s.upsert_agent_config(&cfg).await.unwrap();
    let reloaded = s.get_agent_config("brian").await.unwrap().unwrap();
    assert_eq!(reloaded.model_name, "claude-haiku-4-5");
    assert_eq!(reloaded.auth_token.as_deref(), Some("sk-test-123"));
}

#[tokio::test]
async fn upsert_agent_config_inserts_new_via_constructor() {
    // agent_configs has a CHECK constraint allowing only emma/brian/rain.
    // Use the pre-seeded "rain" row as the canonical upsert target.
    let s = Storage::memory().await.unwrap();
    let cfg = AgentConfig {
        agent_name: "rain".into(),
        provider: "deepseek".into(),
        model_name: "deepseek-coder".into(),
        base_url: Some("https://api.deepseek.com".into()),
        auth_token: Some("ds-token".into()),
        updated_at: String::new(), // ignored on insert/upsert
    };
    s.upsert_agent_config(&cfg).await.unwrap();
    let got = s.get_agent_config("rain").await.unwrap().unwrap();
    assert_eq!(got.provider, "deepseek");
    assert_eq!(got.model_name, "deepseek-coder");
    assert_eq!(got.base_url.as_deref(), Some("https://api.deepseek.com"));
}

#[tokio::test]
async fn set_session_spawn_models_round_trip() {
    // Sessions remember which model their agents were spawned with so the chat
    // header reflects what's actually talking, even after a config swap.
    let s = Storage::memory().await.unwrap();
    s.create_session("sess-x", "test", Some("/tmp/repo"))
        .await
        .unwrap();
    let before = s.get_session("sess-x").await.unwrap().unwrap();
    assert!(before.brian_model_at_spawn.is_none());
    assert!(before.rain_model_at_spawn.is_none());

    s.set_session_spawn_models("sess-x", "claude-opus-4-7", "deepseek-v4-pro")
        .await
        .unwrap();
    let after = s.get_session("sess-x").await.unwrap().unwrap();
    assert_eq!(after.brian_model_at_spawn.as_deref(), Some("claude-opus-4-7"));
    assert_eq!(after.rain_model_at_spawn.as_deref(), Some("deepseek-v4-pro"));
}
