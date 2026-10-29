mod roster_boundary {
    use super::*;
    use crate::core::roster_boundary::RosterBoundary;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    #[tokio::test]
    async fn scoped_join_survives_the_hold_and_cannot_replace_new_input_after_retirement() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (bridge, tx, task) = start(deps).await;
        post(&storage, "user", None, "go").await;
        send(&tx, user_message()).await;
        seats[0].expect(1).await;
        let waiter = reserve(&bridge);
        tokio::time::sleep(QUIET).await;
        finish(&tx, seats[0].id, 1).await;
        let boundary = acquired(waiter).await;
        let old = crate::agents::incarnation::Incarnation::new();
        let (obsolete, mut obsolete_seat) = late_stdin(seats[0].id);
        send(
            &tx,
            SequencerCommand::ParticipantJoined {
                participant_id: seats[0].id,
                input: obsolete,
            }
            .from_incarnation(&old),
        )
        .await;
        let (replacement, mut new_seat) = late_stdin(seats[0].id);
        boundary
            .bind(
                seats[0].id,
                Some((replacement, Arc::new(AtomicU64::new(0)))),
            )
            .await
            .unwrap();
        old.retire().await;
        post(&storage, "user", None, "replacement work").await;
        send(&tx, user_message()).await;
        boundary.release().await.unwrap();
        new_seat.expect(1).await;
        obsolete_seat.quiet().await;
        task.abort();
    }

    #[tokio::test]
    async fn scoped_wake_keeps_its_identity_through_pause_and_reservation_replay() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "on_mention")]).await;
        let (bridge, tx, task) = start(deps).await;
        let old = crate::agents::incarnation::Incarnation::new();
        post(&storage, "system", None, "old retry").await;
        send(&tx, SequencerCommand::Pause).await;
        send(
            &tx,
            SequencerCommand::Summon {
                participant_id: seats[1].id,
            }
            .from_incarnation(&old),
        )
        .await;
        // The acknowledgement proves commands ahead of Reserve were consumed;
        // it also moves Pause's held envelope through the reservation queue.
        let boundary = acquired(reserve(&bridge)).await;
        old.retire().await;
        boundary.release().await.unwrap();
        send(&tx, SequencerCommand::Resume).await;
        seats[0].quiet().await;
        seats[1].quiet().await;
        task.abort();
    }

    #[tokio::test]
    async fn cached_process_summons_retire_but_user_mentions_keep_their_authority() {
        for user_mentions_replacement in [false, true] {
            let (deps, storage, mut seats) =
                ring(&[("a", "active"), ("b", "active"), ("c", "on_mention")]).await;
            let (bridge, tx, task) = start(deps).await;
            post(&storage, "user", None, "go").await;
            send(&tx, user_message()).await;
            seats[0].expect(1).await;
            let old = crate::agents::incarnation::Incarnation::new();
            send(
                &tx,
                SequencerCommand::Summon {
                    participant_id: seats[2].id,
                }
                .from_incarnation(&old),
            )
            .await;
            let waiter = reserve(&bridge);
            tokio::time::sleep(QUIET).await;
            finish(&tx, seats[0].id, 1).await;
            let boundary = acquired(waiter).await;
            let (input, mut replacement) = late_stdin(seats[2].id);
            boundary
                .bind(seats[2].id, Some((input, Arc::new(AtomicU64::new(0)))))
                .await
                .unwrap();
            old.retire().await;
            post(&storage, "user", None, "new direction").await;
            send(
                &tx,
                SequencerCommand::UserMessage {
                    mentions: if user_mentions_replacement {
                        vec![seats[2].id]
                    } else {
                        vec![]
                    },
                    restarts_rotation: true,
                    answers_for: vec![],
                },
            )
            .await;
            boundary.release().await.unwrap();
            if user_mentions_replacement {
                replacement.expect(2).await;
                seats[0].quiet().await;
            } else {
                seats[0].expect(1).await;
                replacement.quiet().await;
            }
            seats[1].quiet().await;
            seats[2].quiet().await;
            task.abort();
        }
    }

    #[tokio::test]
    async fn a_paused_retired_completion_cannot_end_the_replacements_matching_epoch() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (_bridge, tx, task) = start(deps).await;
        post(&storage, "user", None, "go").await;
        send(&tx, user_message()).await;
        seats[0].expect(1).await;
        let old = crate::agents::incarnation::Incarnation::new();
        send(&tx, SequencerCommand::Pause).await;
        send(
            &tx,
            SequencerCommand::TurnComplete {
                participant_id: seats[0].id,
                epoch: 1,
                ending: DONE,
            }
            .from_incarnation(&old),
        )
        .await;
        seats[1].quiet().await;
        old.retire().await;
        send(&tx, SequencerCommand::Resume).await;
        seats[1].quiet().await;
        assert!(
            !storage
                .participant_by_id(seats[0].id)
                .await
                .unwrap()
                .unwrap()
                .done_vote
        );
        finish(&tx, seats[0].id, 1).await;
        seats[1].expect(1).await;
        task.abort();
    }

    async fn start(
        mut deps: SequencerDeps,
    ) -> (
        Arc<SignalingBridge>,
        mpsc::Sender<SequencerCommand>,
        JoinHandle<()>,
    ) {
        let bridge = SignalingBridge::new();
        bridge.set_storage(deps.storage.clone()).await;
        deps.bridge = Some(bridge.clone());
        let (tx, rx) = mpsc::channel(64);
        bridge
            .register_session_sequencer("s1".into(), tx.clone())
            .await;
        (bridge, tx, tokio::spawn(run_sequencer(deps, rx)))
    }

    fn reserve(bridge: &Arc<SignalingBridge>) -> JoinHandle<RosterBoundary> {
        let bridge = bridge.clone();
        tokio::spawn(async move {
            bridge
                .reserve_roster_boundary("s1", "change-1")
                .await
                .unwrap()
        })
    }

    async fn acquired(waiter: JoinHandle<RosterBoundary>) -> RosterBoundary {
        tokio::time::timeout(DEADLINE, waiter)
            .await
            .expect("boundary never acknowledged")
            .expect("reservation panicked")
    }

    async fn finish(tx: &mpsc::Sender<SequencerCommand>, id: i64, epoch: u64) {
        send(
            tx,
            SequencerCommand::TurnComplete {
                participant_id: id,
                epoch,
                ending: SPOKE,
            },
        )
        .await;
    }

    #[tokio::test]
    async fn live_turn_then_atomic_tool_block_ack_and_next_deal_until_release() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let atomic = deps.in_atomic_tool.clone();
        let (bridge, tx, task) = start(deps).await;
        post(&storage, "user", None, "go").await;
        send(&tx, user_message()).await;
        seats[0].expect(1).await;
        let mut waiter = reserve(&bridge);
        assert!(tokio::time::timeout(QUIET, &mut waiter).await.is_err());
        atomic.store(true, Ordering::Release);
        finish(&tx, seats[0].id, 1).await;
        assert!(tokio::time::timeout(QUIET, &mut waiter).await.is_err());
        seats[1].quiet().await;
        atomic.store(false, Ordering::Release);
        let boundary = acquired(waiter).await;
        let holder: Option<i64> =
            sqlx::query_scalar("SELECT current_turn_participant_id FROM sessions WHERE id = 's1'")
                .fetch_one(storage.pool())
                .await
                .unwrap();
        assert_eq!(holder, None);
        seats[1].quiet().await;
        boundary.release().await.unwrap();
        seats[1].expect(1).await;
        seats[0].quiet().await;
        task.abort();
    }

    #[tokio::test]
    async fn idle_release_does_not_invent_a_turn() {
        let (deps, _storage, mut seats) = ring(&[("a", "active")]).await;
        let (bridge, _tx, task) = start(deps).await;
        acquired(reserve(&bridge)).await.release().await.unwrap();
        seats[0].quiet().await;
        task.abort();
    }

    #[tokio::test]
    async fn stage_during_the_hold_delivers_at_this_boundary_before_the_next_peer() {
        for pause in [false, true] {
            let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
            let (bridge, tx, task) = start(deps).await;
            let mut events = bridge.subscribe();
            post(&storage, "user", None, "go").await;
            send(&tx, user_message()).await;
            seats[0].expect(1).await;
            let waiter = reserve(&bridge);
            tokio::time::sleep(QUIET).await;
            finish(&tx, seats[0].id, 1).await;
            let boundary = acquired(waiter).await;
            send(&tx, SequencerCommand::MessageStaged).await;
            if pause {
                send(&tx, SequencerCommand::Pause).await;
            }
            boundary.release().await.unwrap();
            if pause {
                assert!(next_due_quick(&mut events).await.is_none());
                send(&tx, SequencerCommand::Resume).await;
            }
            assert_eq!(next_due(&mut events).await.as_deref(), Some("s1"));
            seats[1].quiet().await;
            post(&storage, "user", None, "staged direction").await;
            send(&tx, user_message()).await;
            seats[0].expect(1).await;
            task.abort();
        }
    }

    #[tokio::test]
    async fn stage_then_unstage_during_an_idle_hold_creates_no_work() {
        let (deps, _storage, mut seats) = ring(&[("a", "active")]).await;
        let (bridge, tx, task) = start(deps).await;
        let mut events = bridge.subscribe();
        let boundary = acquired(reserve(&bridge)).await;
        send(&tx, SequencerCommand::MessageStaged).await;
        send(&tx, SequencerCommand::MessageUnstaged).await;
        boundary.release().await.unwrap();
        assert!(next_due_quick(&mut events).await.is_none());
        seats[0].quiet().await;
        task.abort();
    }

    #[tokio::test]
    async fn an_answer_already_retained_before_reserving_keeps_the_anchor() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (bridge, tx, task) = start(deps).await;
        post(&storage, "user", None, "go").await;
        send(&tx, user_message()).await;
        seats[0].expect(1).await;
        post(&storage, "user", None, "tray answer").await;
        send(&tx, answer_message()).await;
        let waiter = reserve(&bridge);
        tokio::time::sleep(QUIET).await;
        finish(&tx, seats[0].id, 1).await;
        let boundary = acquired(waiter).await;
        boundary.release().await.unwrap();
        seats[1].expect(2).await;
        seats[0].quiet().await;
        task.abort();
    }

    #[tokio::test]
    async fn a_winding_down_halt_waits_for_completion_not_its_old_grace_timer() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (bridge, tx, task) = start(deps).await;
        post(&storage, "user", None, "go").await;
        send(&tx, user_message()).await;
        seats[0].expect(1).await;
        send(&tx, halt_by(seats[0].id)).await;
        post(&storage, "user", None, "answer").await;
        send(&tx, answer_message()).await;
        let mut waiter = reserve(&bridge);
        assert!(
            tokio::time::timeout(HALT_WIND_DOWN_GRACE + QUIET, &mut waiter)
                .await
                .is_err()
        );
        seats[1].quiet().await;
        finish(&tx, seats[0].id, 1).await;
        let boundary = acquired(waiter).await;
        boundary.release().await.unwrap();
        seats[1].expect(2).await;
        task.abort();
    }

    #[tokio::test]
    async fn halt_arriving_during_the_reservation_does_not_rearm_completed_wind_down() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (bridge, tx, task) = start(deps).await;
        post(&storage, "user", None, "go").await;
        send(&tx, user_message()).await;
        seats[0].expect(1).await;
        let waiter = reserve(&bridge);
        tokio::time::sleep(QUIET).await;
        send(&tx, halt_by(seats[0].id)).await;
        finish(&tx, seats[0].id, 1).await;
        let boundary = acquired(waiter).await;
        post(&storage, "user", None, "continue").await;
        send(&tx, answer_message()).await;
        boundary.release().await.unwrap();
        seats[1].expect(2).await;
        seats[0].quiet().await;
        task.abort();
    }

    #[tokio::test]
    async fn a_paused_user_release_replays_before_ack_and_waits_for_install_to_deal() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (bridge, tx, task) = start(deps).await;
        post(&storage, "user", None, "go").await;
        send(&tx, user_message()).await;
        seats[0].expect(1).await;
        send(&tx, SequencerCommand::Pause).await;
        let mut waiter = reserve(&bridge);
        assert!(tokio::time::timeout(QUIET, &mut waiter).await.is_err());
        post(&storage, "user", None, "after pause").await;
        send(&tx, user_message()).await;
        assert!(tokio::time::timeout(QUIET, &mut waiter).await.is_err());
        finish(&tx, seats[0].id, 1).await;
        let boundary = acquired(waiter).await;
        seats[0].quiet().await;
        boundary.release().await.unwrap();
        seats[0].expect(1).await;
        seats[1].quiet().await;
        task.abort();
    }

    #[tokio::test]
    async fn replacement_installs_input_and_epoch_and_ignores_retired_completion() {
        let (mut deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let old_epoch = Arc::new(AtomicU64::new(0));
        deps.epochs.insert(seats[0].id, old_epoch.clone());
        let (bridge, tx, task) = start(deps).await;
        post(&storage, "user", None, "start").await;
        send(&tx, user_message()).await;
        seats[0].expect(1).await;
        let waiter = reserve(&bridge);
        tokio::time::sleep(QUIET).await;
        finish(&tx, seats[0].id, 1).await;
        let boundary = acquired(waiter).await;
        let (input, mut replacement) = late_stdin(seats[0].id);
        let epoch = Arc::new(AtomicU64::new(999));
        boundary
            .bind(seats[0].id, Some((input, epoch.clone())))
            .await
            .unwrap();
        assert_eq!(epoch.load(Ordering::Acquire), 0);
        finish(&tx, seats[0].id, 1).await;
        post(&storage, "user", None, "new direction").await;
        send(&tx, user_message()).await;
        replacement.quiet().await;
        boundary.release().await.unwrap();
        assert_eq!(replacement.expect(1).await, vec!["new direction"]);
        assert_eq!(epoch.load(Ordering::Acquire), 3);
        assert_eq!(old_epoch.load(Ordering::Acquire), 1);
        seats[1].quiet().await;
        seats[0].quiet().await;
        task.abort();
    }

    #[tokio::test]
    async fn answer_then_typed_mentions_are_retained_and_deal_once() {
        let (deps, storage, mut seats) =
            ring(&[("a", "active"), ("b", "active"), ("c", "on_mention")]).await;
        let (bridge, tx, task) = start(deps).await;
        let boundary = acquired(reserve(&bridge)).await;
        post(&storage, "user", None, "answer").await;
        send(&tx, answer_message()).await;
        post(&storage, "user", None, "@c inspect").await;
        send(
            &tx,
            SequencerCommand::UserMessage {
                mentions: vec![seats[2].id],
                restarts_rotation: true,
                answers_for: vec![],
            },
        )
        .await;
        boundary.release().await.unwrap();
        seats[2].expect(2).await;
        seats[0].quiet().await;
        seats[1].quiet().await;
        task.abort();
    }

    #[tokio::test]
    async fn pause_catches_completion_but_does_not_prevent_safe_reservation() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "active")]).await;
        let (bridge, tx, task) = start(deps).await;
        post(&storage, "user", None, "go").await;
        send(&tx, user_message()).await;
        seats[0].expect(1).await;
        send(&tx, SequencerCommand::Pause).await;
        finish(&tx, seats[0].id, 1).await;
        let boundary = acquired(reserve(&bridge)).await;
        boundary.release().await.unwrap();
        seats[1].quiet().await;
        send(&tx, SequencerCommand::Resume).await;
        seats[1].expect(1).await;
        task.abort();
    }

    #[tokio::test]
    async fn pause_after_queued_steer_wins_until_user_resumes() {
        let (deps, storage, mut seats) = ring(&[("a", "active")]).await;
        let (bridge, tx, task) = start(deps).await;
        let boundary = acquired(reserve(&bridge)).await;
        post(&storage, "user", None, "steer").await;
        send(&tx, user_message()).await;
        send(&tx, SequencerCommand::Pause).await;
        boundary.release().await.unwrap();
        seats[0].quiet().await;
        send(&tx, SequencerCommand::Resume).await;
        seats[0].expect(1).await;
        task.abort();
    }

    #[tokio::test]
    async fn cancelled_pending_waiter_replays_and_does_not_leave_a_hold() {
        let (mut deps, storage, mut seats) = ring(&[("a", "active")]).await;
        deps.in_atomic_tool = Arc::new(AtomicBool::new(true));
        let (bridge, tx, task) = start(deps).await;
        let mut waiter = reserve(&bridge);
        assert!(tokio::time::timeout(QUIET, &mut waiter).await.is_err());
        post(&storage, "user", None, "resume work").await;
        send(&tx, user_message()).await;
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        seats[0].expect(1).await;
        task.abort();
    }

    #[tokio::test]
    async fn abandoned_granted_owner_stops_with_a_concrete_recap() {
        let (deps, storage, mut seats) = ring(&[("a", "active")]).await;
        let (bridge, tx, task) = start(deps).await;
        drop(acquired(reserve(&bridge)).await);
        tokio::time::timeout(DEADLINE, async {
            loop {
                if notices(&storage)
                    .await
                    .iter()
                    .any(|n| n.contains("change-1 lost its worker"))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        post(&storage, "user", None, "go").await;
        send(&tx, user_message()).await;
        seats[0].quiet().await;
        assert!(bridge
            .reserve_roster_boundary("s1", "change-2")
            .await
            .is_err());
        task.abort();
    }

    #[tokio::test]
    async fn stale_release_cannot_release_another_boundary() {
        let (deps, storage, mut seats) = ring(&[("a", "active")]).await;
        let (bridge, tx, task) = start(deps).await;
        let boundary = acquired(reserve(&bridge)).await;
        let (reply, ack) = tokio::sync::oneshot::channel();
        send(
            &tx,
            SequencerCommand::RosterBoundary(BoundaryCommand::Release {
                token: "old-owner".into(),
                reply,
            }),
        )
        .await;
        assert!(ack.await.unwrap().is_err());
        post(&storage, "user", None, "go").await;
        send(&tx, user_message()).await;
        seats[0].quiet().await;
        boundary.release().await.unwrap();
        seats[0].expect(1).await;
        task.abort();
    }

    #[tokio::test]
    async fn an_open_gate_still_holds_a_summons_after_the_boundary_releases() {
        let (deps, storage, mut seats) = ring(&[("a", "active"), ("b", "on_mention")]).await;
        let (bridge, tx, task) = start(deps).await;
        let boundary = acquired(reserve(&bridge)).await;
        post(&storage, "system", None, "review this").await;
        send(&tx, gate_opened("gate-1")).await;
        send(
            &tx,
            SequencerCommand::Summon {
                participant_id: seats[1].id,
            },
        )
        .await;
        boundary.release().await.unwrap();
        seats[1].quiet().await;
        send(&tx, gate_resolved("gate-1")).await;
        seats[1].expect(1).await;
        seats[0].quiet().await;
        task.abort();
    }

    #[tokio::test]
    async fn a_declared_halt_survives_a_boundary_and_summons_until_the_user_answers() {
        let (deps, storage, mut seats) = ring(&[("a", "active")]).await;
        let (bridge, tx, task) = start(deps).await;
        let boundary = acquired(reserve(&bridge)).await;
        send(&tx, halt_by(seats[0].id)).await;
        post(&storage, "system", None, "queued work").await;
        send(
            &tx,
            SequencerCommand::Summon {
                participant_id: seats[0].id,
            },
        )
        .await;
        boundary.release().await.unwrap();
        seats[0].quiet().await;
        post(&storage, "user", None, "answer").await;
        send(&tx, answer_message()).await;
        seats[0].expect(2).await;
        task.abort();
    }

    #[tokio::test]
    async fn a_later_halt_is_not_cleared_by_an_earlier_queued_user_message() {
        let (deps, storage, mut seats) = ring(&[("a", "active")]).await;
        let (bridge, tx, task) = start(deps).await;
        let boundary = acquired(reserve(&bridge)).await;
        post(&storage, "user", None, "earlier steer").await;
        send(&tx, user_message()).await;
        send(&tx, halt_by(seats[0].id)).await;
        boundary.release().await.unwrap();
        seats[0].quiet().await;
        post(&storage, "user", None, "later answer").await;
        send(&tx, answer_message()).await;
        seats[0].expect(2).await;
        task.abort();
    }

    #[tokio::test]
    async fn boot_blocks_ack_but_unrelated_late_orientation_does_not() {
        let (mut deps, _storage, _seats) = ring(&[("a", "active")]).await;
        deps.boots = true;
        deps.orienting = Some(seed_orienting(true, [1]));
        let (bridge, tx, task) = start(deps).await;
        let mut waiter = reserve(&bridge);
        assert!(tokio::time::timeout(QUIET, &mut waiter).await.is_err());
        send(&tx, SequencerCommand::BootEnded).await;
        acquired(waiter).await.release().await.unwrap();
        task.abort();
    }

    #[tokio::test]
    async fn removing_the_completed_anchor_steps_onward_without_reset() {
        let (deps, storage, mut seats) =
            ring(&[("a", "active"), ("b", "active"), ("c", "active")]).await;
        let (bridge, tx, task) = start(deps).await;
        post(&storage, "user", None, "go").await;
        send(&tx, user_message()).await;
        seats[0].expect(1).await;
        finish(&tx, seats[0].id, 1).await;
        seats[1].expect(1).await;
        let waiter = reserve(&bridge);
        tokio::time::sleep(QUIET).await;
        finish(&tx, seats[1].id, 2).await;
        let boundary = acquired(waiter).await;
        sqlx::query("UPDATE session_participants SET enabled = 0 WHERE id = ?")
            .bind(seats[1].id)
            .execute(storage.pool())
            .await
            .unwrap();
        boundary.bind(seats[1].id, None).await.unwrap();
        boundary.release().await.unwrap();
        seats[2].expect(1).await;
        seats[0].quiet().await;
        task.abort();
    }

    #[tokio::test]
    async fn close_during_reservation_refuses_binding_and_release_delivery() {
        let (deps, storage, mut seats) = ring(&[("a", "active")]).await;
        let (bridge, tx, task) = start(deps).await;
        let boundary = acquired(reserve(&bridge)).await;
        post(&storage, "user", None, "go").await;
        send(&tx, user_message()).await;
        storage.close_session("s1", false).await.unwrap();
        assert!(boundary.bind(seats[0].id, None).await.is_err());
        assert!(boundary.release().await.is_err());
        seats[0].quiet().await;
        task.abort();
    }
}
