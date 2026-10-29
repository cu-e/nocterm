use crate::{CloudChat, CloudChatError, PendingCommandState};
use gpui::{Entity, TestAppContext};
use nocterm_agent_chat::conversation::{ConversationActions, ConversationError};
use nocterm_agent_runtime::history_admission::{
    HistoryAdmissionHandle, MAX_NORMAL_HISTORY_PAYLOADS, testing::HistoryProbe,
};
use nocterm_ai::{conversation::ConversationAction, thread::Entry};
use nocterm_cloud_model::projection::{
    ProjectionError,
    testing::{ProjectionFixture, ScriptedConversationSource, prompt_event, run_binding},
};
use std::{rc::Rc, sync::Arc};

fn installation(
    cx: &mut TestAppContext,
) -> (
    Arc<ScriptedConversationSource>,
    crate::CloudChatInstallation,
    Rc<dyn nocterm_agent_chat::conversation::ConversationSource>,
) {
    let fixture = ProjectionFixture::new();
    let source = Arc::new(ScriptedConversationSource::new(fixture.binding().clone()));
    let installed =
        cx.update(|cx| crate::CloudChatInstallation::new(HistoryProbe::installation(), cx));
    let view = cx.update(|cx| installed.open(source.clone(), cx).unwrap());
    cx.run_until_parked();
    (source, installed, view)
}

fn view_phase(
    view: &dyn nocterm_agent_chat::conversation::ConversationSource,
    cx: &mut TestAppContext,
) -> nocterm_ai::conversation::ConversationPhase {
    let mut phase = None;
    cx.update(|cx| {
        view.with_view(cx, &mut |view| phase = Some(view.phase))
            .unwrap()
    });
    phase.unwrap()
}

#[gpui::test]
fn installation_window_observers_share_replay_and_only_the_last_detach_pauses_it(
    cx: &mut TestAppContext,
) {
    use nocterm_ai::conversation::ConversationPhase;
    use nocterm_cloud_model::coordinator::{CoordinatorEvent, RunStatus};
    let (source, installed, first) = installation(cx);
    let second = cx.update(|cx| installed.open(source.clone(), cx).unwrap());
    cx.run_until_parked();
    assert_eq!(source.effects().replayed.len(), 1);
    cx.update(|cx| first.set_shown(false, cx).unwrap());
    let mut fixture = ProjectionFixture::new();
    source
        .queue(fixture.page(vec![prompt_event(
            &run_binding(1),
            "shared",
            nocterm_cloud_model::identity::CommandId::from_bytes([77; 16]),
        )]))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(view_phase(first.as_ref(), cx), ConversationPhase::Queued);
    assert_eq!(view_phase(second.as_ref(), cx), ConversationPhase::Queued);
    cx.update(|_| drop(second));
    cx.run_until_parked();
    source
        .queue(fixture.page(vec![CoordinatorEvent::Status {
            run: run_binding(1),
            status: RunStatus::Stopped,
        }]))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(view_phase(first.as_ref(), cx), ConversationPhase::Queued);
    cx.update(|cx| first.set_shown(true, cx).unwrap());
    cx.run_until_parked();
    assert_eq!(view_phase(first.as_ref(), cx), ConversationPhase::Stopped);
    assert!(source.effects().staged.is_empty());
}

#[gpui::test]
async fn installation_keeps_a_staged_command_alive_after_its_last_window_closes(
    cx: &mut TestAppContext,
) {
    let (source, installed, view) = installation(cx);
    let delivery = source.pause_delivery();
    let controls = cx.update(|cx| view.capture_actions(cx).unwrap());
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("retained".into()), cx));
    cx.run_until_parked();
    assert_eq!(source.effects().staged.len(), 1);
    cx.update(|_| drop((view, controls)));
    cx.run_until_parked();
    delivery.release();
    sent.await.unwrap();
    assert_eq!(source.effects().delivered, source.effects().staged);
    let reopened = cx.update(|cx| installed.open(source.clone(), cx).unwrap());
    cx.update(|cx| {
        reopened
            .with_view(cx, &mut |view| assert_eq!(view.pending_commands, 1))
            .unwrap()
    });
    assert_eq!(source.effects().staged.len(), 1);
}

#[gpui::test]
async fn installation_same_account_login_creates_a_new_capture_without_retargeting_old_views(
    cx: &mut TestAppContext,
) {
    let (old, installed, view) = installation(cx);
    let controls = cx.update(|cx| view.capture_actions(cx).unwrap());
    let fresh = Arc::new(old.replace_membership());
    let next = cx.update(|cx| installed.open(fresh.clone(), cx).unwrap());
    let refused = cx.update(|cx| controls.perform(ConversationAction::Prompt("old".into()), cx));
    assert!(matches!(refused.await, Err(ConversationError::Stale)));
    cx.update(|cx| {
        assert!(matches!(
            view.with_view(cx, &mut |_| {}),
            Err(ConversationError::Stale)
        ))
    });
    let next_controls = cx.update(|cx| next.capture_actions(cx).unwrap());
    cx.update(|cx| next_controls.perform(ConversationAction::Prompt("new".into()), cx))
        .await
        .unwrap();
    assert_eq!(fresh.effects().staged.len(), 1);
    let id = fresh.effects().staged[0];
    assert_eq!(
        fresh.staged(id).unwrap().action,
        ConversationAction::Prompt("new".into())
    );
}

#[gpui::test]
fn installation_observer_capacity_is_reclaimed_only_after_the_last_rc_drops(
    cx: &mut TestAppContext,
) {
    let (source, installed, first) = installation(cx);
    let mut views = (1..256)
        .map(|_| cx.update(|cx| installed.open(source.clone(), cx).unwrap()))
        .collect::<Vec<_>>();
    assert!(cx.update(|cx| installed.open(source.clone(), cx)).is_err());
    let retained = views.pop().unwrap();
    let shared = retained.clone();
    cx.update(|_| drop(retained));
    cx.run_until_parked();
    assert!(cx.update(|cx| installed.open(source.clone(), cx)).is_err());
    cx.update(|_| drop(shared));
    cx.run_until_parked();
    let replacement = cx.update(|cx| installed.open(source.clone(), cx).unwrap());
    assert_eq!(source.effects().replayed.len(), 1);
    drop((first, replacement, views));
}

fn opened(
    cx: &mut TestAppContext,
) -> (
    Arc<ScriptedConversationSource>,
    Entity<CloudChat>,
    HistoryAdmissionHandle,
) {
    let fixture = ProjectionFixture::new();
    let source = Arc::new(ScriptedConversationSource::new(fixture.binding().clone()));
    let admission = HistoryProbe::installation();
    let chat = cx.update(|cx| CloudChat::open(source.clone(), admission.clone(), cx).unwrap());
    cx.run_until_parked();
    (source, chat, admission)
}
fn actions(chat: &Entity<CloudChat>, cx: &mut TestAppContext) -> Rc<dyn ConversationActions> {
    cx.update(|cx| CloudChat::conversation(chat).capture_actions(cx).unwrap())
}

#[gpui::test]
async fn lost_ack_retries_the_original_id_and_accepted_replay_adds_one_user_row(
    cx: &mut TestAppContext,
) {
    let (source, chat, _) = opened(cx);
    let controls = actions(&chat, cx);
    source.lose_next_ack();
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("question".into()), cx));
    assert!(matches!(sent.await, Err(ConversationError::OutcomeUnknown)));
    let pending = cx.update(|cx| chat.read(cx).pending_commands());
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].state, PendingCommandState::OutcomeUnknown);
    let id = pending[0].id;
    assert_eq!(
        source.staged(id).unwrap().action,
        ConversationAction::Prompt("question".into())
    );
    let retried = chat.update(cx, |chat, cx| chat.retry(id, cx));
    retried.await.unwrap();
    assert_eq!(source.effects().staged, [id]);
    assert_eq!(source.effects().delivered, [id]);
    assert_eq!(source.effects().queried, [id]);
    let mut fixture = ProjectionFixture::new();
    let page = fixture.page(vec![prompt_event(&run_binding(1), "question", id)]);
    source.queue(page.clone()).unwrap();
    cx.run_until_parked();
    source.queue(page).unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let chat = chat.read(cx);
        assert!(chat.pending_commands().is_empty());
        assert_eq!(chat.projection().transcript().entries.len(), 1);
        assert!(matches!(
            &chat.projection().transcript().entries[0],
            Entry::User(_)
        ));
    });
}

#[gpui::test]
async fn same_account_replacement_before_first_poll_prevents_outbox_and_delivery(
    cx: &mut TestAppContext,
) {
    let (source, chat, _) = opened(cx);
    let controls = actions(&chat, cx);
    let sent =
        cx.update(|cx| controls.perform(ConversationAction::Prompt("old account".into()), cx));
    let next = source.replace_membership();
    let _again = next.replace_membership(); // A -> B -> A still ends original A.
    cx.run_until_parked();
    assert!(matches!(sent.await, Err(ConversationError::Stale)));
    assert!(source.effects().staged.is_empty());
    assert!(source.effects().delivered.is_empty());
    cx.update(|cx| {
        let chat = chat.read(cx);
        assert!(chat.pending_commands().is_empty());
        assert!(chat.projection().transcript().entries.is_empty());
        assert!(matches!(
            chat.last_error(),
            Some(CloudChatError::Source(ProjectionError::Stale))
        ));
    });
}

#[gpui::test]
async fn replaced_source_cannot_publish_a_ready_old_stage_or_cache(cx: &mut TestAppContext) {
    let (source, chat, _) = opened(cx);
    let stage = source.pause_stage();
    let cache = source.pause_cache();
    let mut fixture = ProjectionFixture::new();
    source
        .queue(fixture.page(vec![prompt_event(
            &run_binding(1),
            "accepted old history",
            nocterm_cloud_model::identity::CommandId::from_bytes([9; 16]),
        )]))
        .unwrap();
    cx.run_until_parked();
    let controls = actions(&chat, cx);
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("old stage".into()), cx));
    cx.run_until_parked();
    stage.release();
    cache.release();
    let _new = source.replace_membership();
    cx.run_until_parked();
    assert!(matches!(sent.await, Err(ConversationError::Stale)));
    assert!(source.effects().staged.is_empty());
    assert!(source.effects().cached.is_empty());
    cx.update(|cx| assert!(chat.read(cx).projection().transcript().entries.is_empty()));
}

#[gpui::test]
async fn dropping_an_action_waiter_does_not_cancel_its_owner_held_delivery(
    cx: &mut TestAppContext,
) {
    let (source, chat, _) = opened(cx);
    let delivery = source.pause_delivery();
    let controls = actions(&chat, cx);
    let waiter = cx
        .update(|cx| controls.perform(ConversationAction::Prompt("retained operation".into()), cx));
    cx.run_until_parked();
    assert_eq!(source.effects().staged.len(), 1);
    drop(waiter);
    delivery.release();
    cx.run_until_parked();
    let effects = source.effects();
    assert_eq!(effects.delivered, effects.staged);
    cx.update(|cx| {
        assert_eq!(
            chat.read(cx).pending_commands()[0].state,
            PendingCommandState::Accepted
        )
    });
}

#[gpui::test]
async fn closing_a_viewer_detaches_observation_without_sending_stop(cx: &mut TestAppContext) {
    let (source, chat, _) = opened(cx);
    let controls = actions(&chat, cx);
    let sent = cx.update(|cx| {
        controls.perform(
            ConversationAction::Prompt("continue independently".into()),
            cx,
        )
    });
    sent.await.unwrap();
    let effects = source.effects();
    cx.update(|cx| CloudChat::conversation(&chat).set_shown(false, cx).unwrap());
    cx.run_until_parked();
    assert_eq!(source.effects().delivered, effects.delivered);
    assert!(
        source
            .effects()
            .staged
            .iter()
            .all(|id| source.staged(*id).unwrap().action != ConversationAction::Stop)
    );
    let _new = source.replace_membership();
    cx.run_until_parked();
    cx.update(|cx| assert!(chat.read(cx).pending_commands().is_empty()));
}

#[gpui::test]
fn cloud_admission_reclaims_a_hidden_neighbor_payload_on_the_shared_installation(
    cx: &mut TestAppContext,
) {
    let admission = HistoryProbe::installation();
    let neighbor = HistoryProbe::default();
    neighbor.set_history(nocterm_agent_runtime::HistoryUsage::new(0, 1, false), true);
    let registration = admission.register(neighbor.participant()).unwrap();
    for _ in 0..MAX_NORMAL_HISTORY_PAYLOADS {
        neighbor.retain_payload(admission.reserve_payload().unwrap());
    }
    let fixture = ProjectionFixture::new();
    let source = Arc::new(ScriptedConversationSource::new(fixture.binding().clone()));
    let chat = cx.update(|cx| CloudChat::open(source, admission.clone(), cx).unwrap());
    assert_eq!(neighbor.payload_count(), MAX_NORMAL_HISTORY_PAYLOADS - 1);
    cx.update(|cx| CloudChat::conversation(&chat).set_shown(false, cx).unwrap());
    drop(registration);
    drop(neighbor);
    drop(chat);
}

#[gpui::test]
fn hidden_cloud_payload_is_reclaimed_by_another_source_without_a_borrow_cycle(
    cx: &mut TestAppContext,
) {
    let (_source, chat, admission) = opened(cx);
    cx.update(|cx| CloudChat::conversation(&chat).set_shown(false, cx).unwrap());
    // Cancellation retires the page only when its actual future is dropped.
    cx.run_until_parked();
    let retained = (1..MAX_NORMAL_HISTORY_PAYLOADS)
        .map(|_| admission.reserve_payload().unwrap())
        .collect::<Vec<_>>();
    let incoming = cx.update(|cx| admission.reserve_payload_reclaiming(None, cx).unwrap());
    assert!(admission.reserve_payload().is_err());
    drop(incoming);
    cx.update(|cx| CloudChat::conversation(&chat).set_shown(true, cx).unwrap());
    assert!(admission.reserve_payload().is_err());
    drop(retained);
}

#[gpui::test]
async fn pending_command_generations_share_the_installation_slots_with_projection_and_replay(
    cx: &mut TestAppContext,
) {
    let (source, chat, _) = opened(cx);
    let controls = actions(&chat, cx);
    let command_slots = MAX_NORMAL_HISTORY_PAYLOADS - 3; // projection + page + one native staging work allocation
    for _ in 0..command_slots {
        cx.update(|cx| controls.perform(ConversationAction::Prompt("queued".into()), cx))
            .await
            .unwrap();
    }
    let last =
        cx.update(|cx| controls.perform(ConversationAction::Prompt("over capacity".into()), cx));
    assert!(matches!(last.await, Err(ConversationError::Capacity)));
    assert_eq!(source.effects().staged.len(), command_slots);
}

#[gpui::test]
async fn captured_actions_do_not_retarget_a_newly_displayed_run_or_control_revision(
    cx: &mut TestAppContext,
) {
    let (source, chat, _) = opened(cx);
    let old_controls = actions(&chat, cx);
    let mut fixture = ProjectionFixture::new();
    source
        .queue(fixture.page(vec![prompt_event(
            &run_binding(1),
            "new accepted run",
            nocterm_cloud_model::identity::CommandId::from_bytes([9; 16]),
        )]))
        .unwrap();
    cx.run_until_parked();
    let sent = cx.update(|cx| old_controls.perform(ConversationAction::Stop, cx));
    assert!(matches!(sent.await, Err(ConversationError::Stale)));
    assert!(source.effects().staged.is_empty());
    assert!(source.effects().delivered.is_empty());
}

#[gpui::test]
async fn definitive_rejections_retire_their_bodies_and_leave_room_for_stop(
    cx: &mut TestAppContext,
) {
    let (source, chat, _) = opened(cx);
    let controls = actions(&chat, cx);
    for _ in 0..40 {
        source.reject_next_command();
        let result = cx
            .update(|cx| controls.perform(ConversationAction::Prompt("refused".into()), cx))
            .await;
        assert!(matches!(result, Err(ConversationError::Unavailable)));
        cx.update(|cx| assert!(chat.read(cx).pending_commands().is_empty()));
    }
    cx.update(|cx| controls.perform(ConversationAction::Stop, cx))
        .await
        .unwrap();
    assert_eq!(source.effects().delivered.len(), 41);
}

#[gpui::test]
fn cancelled_observers_leave_native_stage_cache_and_replay_generations_charged(
    cx: &mut TestAppContext,
) {
    let mut fixture = ProjectionFixture::new();
    let source = Arc::new(ScriptedConversationSource::new(fixture.binding().clone()));
    let executor = cx.background_executor.clone();
    source.run_native_workers_with(Arc::new(move |work| {
        executor.spawn(work).detach();
    }));
    let cache = source.pause_cache();
    let stage = source.pause_stage();
    let admission = HistoryProbe::installation();
    let chat = cx.update(|cx| CloudChat::open(source.clone(), admission.clone(), cx).unwrap());
    cx.run_until_parked();
    source
        .queue(fixture.page(vec![prompt_event(
            &run_binding(1),
            "cached original generation",
            nocterm_cloud_model::identity::CommandId::from_bytes([99; 16]),
        )]))
        .unwrap();
    cx.run_until_parked();
    let controls = actions(&chat, cx);
    let sent = cx.update(|cx| {
        controls.perform(
            ConversationAction::Prompt("native staged command".into()),
            cx,
        )
    });
    cx.run_until_parked();
    let _new = source.replace_membership();
    cx.run_until_parked();
    drop(sent);
    // All GUI owners have retired. Three independent native workers still own
    // exactly their original page, projection and command plus two output proofs.
    let retained = (0..MAX_NORMAL_HISTORY_PAYLOADS - 5)
        .map(|_| admission.reserve_payload().unwrap())
        .collect::<Vec<_>>();
    assert!(admission.reserve_payload().is_err());
    cx.update(|cx| {
        assert!(
            admission
                .reserve_unowned_bytes(
                    nocterm_agent_runtime::history_admission::MAX_NORMAL_HISTORY_BYTES,
                    cx
                )
                .is_err()
        )
    });
    cache.release();
    stage.release();
    source.queue(fixture.page(Vec::new())).unwrap();
    cx.run_until_parked();
    drop(retained);
    let all = (0..MAX_NORMAL_HISTORY_PAYLOADS)
        .map(|_| admission.reserve_payload().unwrap())
        .collect::<Vec<_>>();
    cx.update(|cx| {
        drop(
            admission
                .reserve_unowned_bytes(
                    nocterm_agent_runtime::history_admission::MAX_NORMAL_HISTORY_BYTES,
                    cx,
                )
                .unwrap(),
        )
    });
    drop(all);
}

#[gpui::test]
fn publishing_new_projections_counts_old_native_cache_generations_until_their_actual_drop(
    cx: &mut TestAppContext,
) {
    let mut fixture = ProjectionFixture::new();
    let source = Arc::new(ScriptedConversationSource::new(fixture.binding().clone()));
    let executor = cx.background_executor.clone();
    source.run_native_workers_with(Arc::new(move |work| {
        executor.spawn(work).detach();
    }));
    let cache = source.pause_cache();
    let admission = HistoryProbe::installation();
    let chat = cx.update(|cx| CloudChat::open(source.clone(), admission.clone(), cx).unwrap());
    cx.run_until_parked();
    source
        .queue(fixture.page(vec![prompt_event(
            &run_binding(1),
            "first",
            nocterm_cloud_model::identity::CommandId::from_bytes([99; 16]),
        )]))
        .unwrap();
    cx.run_until_parked();
    source
        .queue(fixture.page(vec![
            nocterm_cloud_model::coordinator::CoordinatorEvent::Status {
                run: run_binding(1),
                status: nocterm_cloud_model::coordinator::RunStatus::Running,
            },
        ]))
        .unwrap();
    cx.run_until_parked();
    // Currentprojection, oldcacheprojection and pendingnativepage are distinct.
    let retained = (0..MAX_NORMAL_HISTORY_PAYLOADS - 4)
        .map(|_| admission.reserve_payload().unwrap())
        .collect::<Vec<_>>();
    assert!(admission.reserve_payload().is_err());
    cache.release();
    cx.run_until_parked();
    let released = admission.reserve_payload().unwrap();
    drop((released, retained, chat));
}

#[gpui::test]
async fn full_normal_pending_load_still_admits_stop_for_the_original_run(cx: &mut TestAppContext) {
    let (source, chat, admission) = opened(cx);
    let mut fixture = ProjectionFixture::new();
    source
        .queue(fixture.page(vec![prompt_event(
            &run_binding(1),
            "originalrun",
            nocterm_cloud_model::identity::CommandId::from_bytes([99; 16]),
        )]))
        .unwrap();
    cx.run_until_parked();
    let controls = actions(&chat, cx);
    for _ in 0..MAX_NORMAL_HISTORY_PAYLOADS - 3 {
        cx.update(|cx| controls.perform(ConversationAction::Prompt("normalpending".into()), cx))
            .await
            .unwrap();
    }
    let last_normal = admission.reserve_payload().unwrap();
    assert!(admission.reserve_payload().is_err());
    cx.update(|cx| controls.perform(ConversationAction::Stop, cx))
        .await
        .unwrap();
    let stop = source
        .staged(*source.effects().delivered.last().unwrap())
        .unwrap();
    assert_eq!(stop.action, ConversationAction::Stop);
    assert_eq!(stop.coordinates.run, Some(run_binding(1)));
    drop(last_normal);
}
