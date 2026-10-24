mod incarnation_fence {
    use super::*;

    #[tokio::test]
    async fn a_retired_handles_events_and_teardown_cannot_touch_its_replacement() {
        // Each event gets its own pump: returning on the FIRST retired event
        // must not accidentally make the other branches vacuous.
        let events = vec![
            Some(AgentEvent::Init {
                session_id: Some("old-resume-id".into()),
                api_key_source: None,
            }),
            Some(AgentEvent::Text("old late text".into())),
            Some(AgentEvent::Notice("You've hit your session limit".into())),
            Some(AgentEvent::Health(AgentHealth::Stalled)),
            Some(AgentEvent::Exited("old process stopped".into())),
            Some(AgentEvent::Compacted {
                trigger: Some("auto".into()),
                pre_tokens: Some(100),
                post_tokens: Some(10),
            }),
            Some(AgentEvent::ToolUse {
                id: "old-edit".into(),
                name: "Edit".into(),
                input: serde_json::json!({}),
            }),
            Some(AgentEvent::ToolResult {
                tool_use_id: "old-atomic".into(),
                content: "done".into(),
                is_error: false,
            }),
            Some(turn_end()),
            None, // A closed event stream reaches the independent teardown path.
        ];
        for event in events {
            let case = format!("{event:?}");
            let (storage, state) = setup().await;
            let pid = storage
                .insert_participant("s1", "hands", "HANDS", None, None, "[]", "active", 0)
                .await
                .unwrap();
            let bridge = SignalingBridge::new();
            bridge.set_storage(storage.clone()).await;
            let activity = ActivityTracker::new(
                "s1",
                Arc::new(AtomicBool::new(false)),
                bridge.clone(),
                vec!["hands".into()],
            );
            let (event_tx, event_rx) = mpsc::channel(16);
            let (input_tx, _input_rx) = mpsc::channel(8);
            let (control_tx, _control_rx) = mpsc::channel(8);
            let (kill_tx, _kill_rx) = tokio::sync::oneshot::channel();
            let mut handle = crate::agents::AgentHandle::from_parts(
                "hands".into(),
                "s1",
                event_rx,
                input_tx,
                control_tx,
                kill_tx,
            );
            let atomic = Arc::new(AtomicBool::new(false));
            let (ring_tx, mut ring_rx) = mpsc::channel(8);
            let cfg = PumpConfig {
                participant_id: Some(pid),
                bridge: Some(bridge.clone()),
                activity: Some(activity.clone()),
                in_atomic_tool: Some(atomic.clone()),
                sequencer_tx: Some(ring_tx),
                turn_epoch: Some(Arc::new(std::sync::atomic::AtomicU64::new(1))),
                edits_files: true,
                self_nudges: true,
                ..PumpConfig::new(
                    "s1",
                    "hands",
                    handle.interrupted_epoch(),
                    handle.incarnation(),
                )
            };
            let receiver = std::mem::replace(&mut handle.event_rx, mpsc::channel(1).1);
            let task = tokio::spawn(pump_agent(cfg, receiver, storage.clone(), state));
            event_tx
                .send(AgentEvent::ToolUse {
                    id: "old-atomic".into(),
                    name: "Bash".into(),
                    input: serde_json::json!({"command":"git commit -m fixture"}),
                })
                .await
                .unwrap();
            event_tx
                .send(AgentEvent::Text("old event barrier".into()))
                .await
                .unwrap();
            wait_for_row(&storage, "old event barrier").await;
            assert!(atomic.load(Ordering::Acquire));
            // This is the ACTUAL handle-to-pump join, not a separate test flag.
            handle.retire().await;
            storage
                .set_participant_claude_id(pid, "replacement-resume-id")
                .await
                .unwrap();
            activity.mark_busy_for_epoch("hands", 99);
            let before = storage
                .messages_for_session("s1", None)
                .await
                .unwrap()
                .len();
            let mut signals = bridge.subscribe();
            if let Some(event) = event {
                event_tx.send(event).await.unwrap();
            }
            drop(event_tx);
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap();
            assert!(
                activity.any_busy(),
                "old cleanup cleared replacement busy: {case}"
            );
            assert!(
                atomic.load(Ordering::Acquire),
                "old cleanup cleared replacement atomic flag: {case}"
            );
            assert_eq!(
                storage
                    .participant_by_id(pid)
                    .await
                    .unwrap()
                    .unwrap()
                    .claude_session_id
                    .as_deref(),
                Some("replacement-resume-id"),
                "{case}"
            );
            assert_eq!(
                storage
                    .messages_for_session("s1", None)
                    .await
                    .unwrap()
                    .len(),
                before,
                "{case}"
            );
            assert!(
                storage
                    .context_readings_for_participant("s1", "hands", 10)
                    .await
                    .unwrap()
                    .is_empty(),
                "{case}"
            );
            assert!(
                storage.session_halt("s1").await.unwrap().is_none(),
                "{case}"
            );
            assert!(
                ring_rx.try_recv().is_err(),
                "old completion reached ring: {case}"
            );
            assert!(
                signals.try_recv().is_err(),
                "old health/context/halt signal: {case}"
            );
        }
    }

    #[tokio::test]
    async fn retirement_revokes_a_retry_already_sleeping_outside_the_pump() {
        let (storage, state) = setup().await;
        let (mut cfg, mut ring_rx) = cfg_with_ring("hands");
        cfg.retry_ladder = vec![Duration::from_millis(200)];
        let incarnation = cfg.incarnation.clone();
        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(pump_agent(cfg, rx, storage.clone(), state));
        tx.send(AgentEvent::Text(
            "API Error: 503 Service Unavailable".into(),
        ))
        .await
        .unwrap();
        let mut failed = turn_end();
        if let AgentEvent::TurnComplete { is_error, .. } = &mut failed {
            *is_error = true;
        }
        tx.send(failed).await.unwrap();
        assert!(matches!(
            next_wire(&mut ring_rx).await,
            crate::core::sequencer::SequencerCommand::TurnComplete { .. }
        ));
        incarnation.retire().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(350), ring_rx.recv())
                .await
                .is_err()
        );
        let rows = storage.messages_for_session("s1", None).await.unwrap();
        assert!(!rows
            .iter()
            .any(|r| r.content.contains("previous turn failed")));
        drop(tx);
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
    }
}
