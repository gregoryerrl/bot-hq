mod spawn_drafts {
    use super::*;
    use crate::storage::{ParticipantChangeOrigin, ParticipantChangeRequest};

    #[tokio::test]
    async fn cold_start_recovers_before_roster_selection_and_declares_conflicts() {
        for conflict in [false, true] {
            let s = Storage::memory().await.unwrap();
            for id in ["old", "new", "external"] { s.upsert_model(&fable_row(id)).await.unwrap(); }
            let p = seed_hands_on(&s, "old").await;
            s.set_participant_claude_id(p.id, "old-thread").await.unwrap();
            s.request_participant_change("s1", "change", s.roster_revision("s1").await.unwrap(),
                ParticipantChangeRequest::SwitchModel { participant_id: p.id, model_id: "new".into() },
                ParticipantChangeOrigin::User).await.unwrap();
            s.claim_participant_change("s1", "change").await.unwrap().unwrap();
            s.prepare_participant_change("s1", "change").await.unwrap();
            let bridge = SignalingBridge::new(); bridge.set_storage(s.clone()).await;
            if conflict { s.set_participant_model(p.id, Some("external")).await.unwrap(); }
            let result = recover_participant_change_before_spawn(&s, &bridge, "s1").await;
            let now = s.participant_by_id(p.id).await.unwrap().unwrap();
            if conflict {
                assert!(result.is_err());
                assert_eq!(now.model_id.as_deref(), Some("external"));
                assert!(s.session_halt("s1").await.unwrap().unwrap().1.contains("could not be recovered"));
            } else {
                result.unwrap();
                assert_eq!(now.model_id.as_deref(), Some("old"));
                assert_eq!(now.claude_session_id.as_deref(), Some("old-thread"));
                assert_eq!(s.participant_change("s1", "change").await.unwrap().unwrap().status, "failed");
                let count = s.messages_for_session("s1", None).await.unwrap().len();
                assert_eq!(count, 1);
                recover_participant_change_before_spawn(&s, &bridge, "s1").await.unwrap();
                assert_eq!(s.messages_for_session("s1", None).await.unwrap().len(), count);
                s.request_participant_change("s1", "pending", s.roster_revision("s1").await.unwrap(),
                    ParticipantChangeRequest::SwitchModel { participant_id: p.id, model_id: "new".into() },
                    ParticipantChangeOrigin::User).await.unwrap();
                recover_participant_change_before_spawn(&s, &bridge, "s1").await.unwrap();
                assert_eq!(s.participant_change("s1", "pending").await.unwrap().unwrap().status, "pending");
            }
        }
    }

    #[test]
    fn every_spawn_recovers_before_project_setup_or_roster_reads() {
        let source = include_str!("session.rs");
        let spawn = &source[source.find("async fn spawn_session_handle(").unwrap()..];
        let recovery = spawn.find("recover_participant_change_before_spawn(&storage, &bridge, &session.id).await?").unwrap();
        assert!(recovery < spawn.find("resolve_session_project(").unwrap());
        assert!(recovery < spawn.find(".ensure_session_roster(").unwrap());
        assert!(recovery < spawn.find(".participants_for_session(").unwrap());
    }

    #[tokio::test]
    async fn delayed_spawn_drafts_do_not_write_through_close_or_rollback_on_either_connector() {
        for codex in [false, true] {
            for outcome in ["close", "rollback", "publish"] {
                let s = Storage::memory().await.unwrap();
                let dirs = TempDir::new().unwrap();
                let account = dirs
                    .path()
                    .join("candidate-account")
                    .to_string_lossy()
                    .into_owned();
                s.upsert_model(&fable_row("old")).await.unwrap();
                let mut model = if codex {
                    astra_row("new", &account)
                } else {
                    fable_row("new")
                };
                model.claude_config_dir = Some(account.clone());
                s.upsert_model(&model).await.unwrap();
                let p = seed_hands_on(&s, "old").await;
                s.set_participant_claude_id(p.id, "old-thread")
                    .await
                    .unwrap();
                s.set_account_at_spawn(
                    p.id,
                    "/old-account",
                    Some("old@example.com"),
                    Some("old-org"),
                )
                .await
                .unwrap();
                s.store_participant_prompt(p.id, "old prompt")
                    .await
                    .unwrap();
                s.set_participant_spawn_knobs(p.id, None, None)
                    .await
                    .unwrap();
                let before = s.participant_by_id(p.id).await.unwrap().unwrap();
                s.request_participant_change(
                    "s1",
                    "change",
                    s.roster_revision("s1").await.unwrap(),
                    ParticipantChangeRequest::SwitchModel {
                        participant_id: p.id,
                        model_id: "new".into(),
                    },
                    ParticipantChangeOrigin::User,
                )
                .await
                .unwrap();
                s.claim_participant_change("s1", "change")
                    .await
                    .unwrap()
                    .unwrap();
                let prepared = s.prepare_participant_change("s1", "change").await.unwrap();
                let expected_prepared = prepared.participant.clone();
                let cfg = frozen_model_config(&p.slug, prepared.model.as_ref().unwrap());
                let frozen_model_name = cfg.model_name.clone();
                let paths = Paths::for_data_dir(dirs.path().join("data"));
                let files = TempDir::new().unwrap();
                let file_path = files.path().to_path_buf();
                let bridge = SignalingBridge::new();
                bridge.set_storage(s.clone()).await;
                bridge.register_mcp_token("s1", &p.slug, "installed-token");
                let started = Arc::new(tokio::sync::Notify::new());
                let release = Arc::new(tokio::sync::Notify::new());
                let auth_probe: Box<AuthProbeFn> = {
                    let started = started.clone();
                    let release = release.clone();
                    Box::new(move |_| {
                        let started = started.clone();
                        let release = release.clone();
                        Box::pin(async move {
                            assert!(!codex, "Codex must not use the Claude probe");
                            started.notify_one();
                            release.notified().await;
                            logged_in("candidate@example.com", "candidate-org")
                        })
                    })
                };
                let codex_probe: Box<CodexProbeFn> = {
                    let started = started.clone();
                    let release = release.clone();
                    Box::new(move |_| {
                        let started = started.clone();
                        let release = release.clone();
                        Box::pin(async move {
                            assert!(codex, "Claude must not use the Codex probe");
                            started.notify_one();
                            release.notified().await;
                            crate::agents::codex::probe::CodexProbe {
                                auth: chatgpt_account("candidate@example.com", "candidate-org"),
                                version: "candidate-version".into(),
                                hooks: vec![],
                                models: vec![],
                            }
                        })
                    })
                };
                let task = {
                    let s = s.clone();
                    let bridge = bridge.clone();
                    tokio::spawn(async move {
                        participant_spawn_draft(
                            &s,
                            &prepared.participant,
                            cfg,
                            &paths,
                            &None,
                            "frozen prompt".into(),
                            "127.0.0.1:4242".parse().unwrap(),
                            &file_path,
                            None,
                            &bridge,
                            &*auth_probe,
                            &*codex_probe,
                            crate::claude_config::AgentOverride {
                                effort: Some("low".into()),
                                ..Default::default()
                            },
                        )
                        .await
                    })
                };
                tokio::time::timeout(std::time::Duration::from_secs(2), started.notified())
                    .await
                    .unwrap();
                model.model_name = "edited-after-preparation".into();
                s.upsert_model(&model).await.unwrap();
                match outcome {
                    "close" => s.close_session("s1", false).await.unwrap(),
                    "rollback" => {
                        assert!(s
                            .finish_participant_change("s1", "change", Some("candidate abandoned"))
                            .await
                            .unwrap());
                    }
                    _ => {}
                }
                release.notify_one();
                let mut draft = tokio::time::timeout(std::time::Duration::from_secs(2), task)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                assert_eq!(draft.config.config.model_name, frozen_model_name);
                assert_eq!(draft.config.overrides.effort.as_deref(), Some("low"));
                assert_eq!(
                    draft.metadata.account_dir.as_deref(),
                    Some(account.as_str())
                );
                assert_eq!(
                    draft.metadata.account_email.as_deref(),
                    Some("candidate@example.com")
                );
                assert_eq!(draft.metadata.resume_id, None);
                if codex {
                    assert_eq!(draft.metadata.runtime_version, "candidate-version");
                }
                assert!(std::fs::read_to_string(&draft.config.system_prompt_path)
                    .unwrap()
                    .starts_with("frozen prompt"));
                assert!(bridge.mcp_token_matches("s1", &p.slug, Some("installed-token")));
                assert!(!bridge.mcp_token_matches("s1", &p.slug, Some(&draft.mcp_token)));
                assert!(s.messages_for_session("s1", None).await.unwrap().is_empty());
                assert!(draft.notices.is_empty());
                let now = s.participant_by_id(p.id).await.unwrap().unwrap();
                assert_eq!(
                    now,
                    if outcome == "publish" {
                        expected_prepared
                    } else {
                        before
                    },
                    "late candidate wrote before publication: {codex}/{outcome}"
                );
                let prompt: Option<String> =
                    sqlx::query_scalar("SELECT prompt FROM session_participants WHERE id = ?")
                        .bind(p.id)
                        .fetch_one(s.pool())
                        .await
                        .unwrap();
                assert_eq!(prompt.as_deref(), Some("old prompt"));
                draft.metadata.resume_id = Some("candidate-ready-thread".into());
                assert_eq!(
                    s.publish_participant_spawn("s1", "change", p.id, &draft.metadata)
                        .await
                        .unwrap(),
                    outcome == "publish"
                );
                if outcome == "publish" {
                    let now = s.participant_by_id(p.id).await.unwrap().unwrap();
                    assert_eq!(
                        now.claude_session_id.as_deref(),
                        Some("candidate-ready-thread")
                    );
                    assert_eq!(
                        now.account_email_at_spawn.as_deref(),
                        Some("candidate@example.com")
                    );
                    assert!(now.spawn_knobs_recorded);
                    assert_eq!(
                        s.participant_change("s1", "change")
                            .await
                            .unwrap()
                            .unwrap()
                            .status,
                        "applied"
                    );
                    assert!(!s
                        .publish_participant_spawn("s1", "change", p.id, &draft.metadata)
                        .await
                        .unwrap());
                }
            }
        }
    }

    #[tokio::test]
    async fn a_draft_keeps_account_warnings_for_its_owner_instead_of_posting_them() {
        let s = Storage::memory().await.unwrap();
        s.upsert_model(&fable_row("model")).await.unwrap();
        let p = seed_hands_on(&s, "model").await;
        s.set_account_at_spawn(
            p.id,
            "/recorded-account",
            Some("old@example.com"),
            Some("old-org"),
        )
        .await
        .unwrap();
        let p = reread(&s).await;
        let dirs = TempDir::new().unwrap();
        let paths = Paths::for_data_dir(dirs.path().join("data"));
        let files = TempDir::new().unwrap();
        let bridge = SignalingBridge::new();
        bridge.set_storage(s.clone()).await;
        let probe: Box<AuthProbeFn> =
            Box::new(|_| Box::pin(async { logged_in("now@example.com", "new-org") }));
        let cfg = resolve_participant_config(&s, &p).await;
        let draft = participant_spawn_draft(
            &s,
            &p,
            cfg.clone(),
            &paths,
            &None,
            "prompt".into(),
            "127.0.0.1:4242".parse().unwrap(),
            files.path(),
            None,
            &bridge,
            &*probe,
            &*no_codex_probe(),
            Default::default(),
        )
        .await
        .unwrap();
        assert!(draft
            .notices
            .iter()
            .any(|n| n.contains("old@example.com") && n.contains("now@example.com")));
        assert!(s.messages_for_session("s1", None).await.unwrap().is_empty());
        assert_eq!(
            draft.metadata.account_email.as_deref(),
            Some("old@example.com"),
            "the original pin is retained"
        );
        assert_eq!(
            draft.config.account_email.as_deref(),
            Some("now@example.com"),
            "billing reports the observed identity"
        );
        participant_spawn_config(
            &s,
            &p,
            cfg,
            &paths,
            &None,
            "prompt".into(),
            "127.0.0.1:4242".parse().unwrap(),
            files.path(),
            None,
            &bridge,
            &*probe,
            &*no_codex_probe(),
        )
        .await
        .unwrap();
        assert!(
            s.messages_for_session("s1", None)
                .await
                .unwrap()
                .iter()
                .any(|m| m.content.contains("now@example.com")),
            "ordinary startup still announces the mismatch"
        );
    }
}
