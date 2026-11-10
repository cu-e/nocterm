use futures::channel::oneshot;
use nocterm_ai::model::session::{SessionUpdate, StopReason, TurnActivity};
use nocterm_hub_proto::v2::{self, chat_item::Body};

use super::{ChatState, Output};

fn item(seq: u64, run: &str, body: Body) -> v2::ChatItem {
    v2::ChatItem {
        seq,
        at_ms: 0,
        run_id: run.into(),
        body: Some(body),
    }
}

fn user(seq: u64, client_message: &str, text: &str) -> v2::ChatItem {
    item(
        seq,
        "",
        Body::User(v2::UserMessage {
            text: text.into(),
            client_message_id: client_message.into(),
            ..Default::default()
        }),
    )
}

fn ended(seq: u64, run: &str, status: v2::RunStatus) -> v2::ChatItem {
    item(
        seq,
        run,
        Body::RunEnded(v2::RunEnded {
            status: status.into(),
            ..Default::default()
        }),
    )
}

fn text(seq: u64, run: &str, text: &str) -> v2::ChatItem {
    item(seq, run, Body::Agent(v2::AgentText { text: text.into() }))
}

fn chunks(outputs: &[Output]) -> Vec<String> {
    outputs
        .iter()
        .filter_map(|output| match output {
            Output::Notice(update) => match update.as_ref() {
                SessionUpdate::AgentMessageChunk(nocterm_ai::model::content::Content::Text(
                    text,
                )) => Some(text.text.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[test]
fn a_prompt_ends_with_the_run_that_started_after_it() {
    let mut chat = ChatState::default();
    chat.item(&text(1, "r0", "earlier run"));
    let (done, mut answer) = oneshot::channel();
    chat.expect("m1".into(), done);
    chat.item(&user(2, "m1", "hello"));
    let outputs = chat.item(&ended(3, "r0", v2::RunStatus::Completed));
    assert!(matches!(
        outputs.last(),
        Some(Output::Notice(update)) if matches!(**update, SessionUpdate::Turn(TurnActivity::Ended))
    ));
    assert!(answer.try_recv().unwrap().is_none());
    chat.item(&text(4, "r1", "hi"));
    let outputs = chat.item(&ended(5, "r1", v2::RunStatus::Stopped));
    let Some(Output::Resolve(sender, result)) = outputs.into_iter().last() else {
        panic!("the prompt resolves");
    };
    let _ = sender.send(result);
    let outcome = answer.try_recv().unwrap().unwrap().unwrap();
    assert_eq!(outcome.stop_reason, StopReason::Cancelled);
}

#[test]
fn a_message_from_another_device_starts_and_ends_a_turn() {
    let mut chat = ChatState::default();
    let outputs = chat.item(&user(1, "other-device", "deploy"));
    assert!(matches!(
        outputs.as_slice(),
        [Output::Notice(update)]
            if matches!(&**update, SessionUpdate::Turn(TurnActivity::Started { prompt }) if prompt.len() == 1)
    ));
    chat.item(&text(2, "r1", "done"));
    assert!(chat.running());
    let outputs = chat.item(&ended(3, "r1", v2::RunStatus::Completed));
    assert!(matches!(
        outputs.as_slice(),
        [Output::Notice(update)] if matches!(**update, SessionUpdate::Turn(TurnActivity::Ended))
    ));
    assert!(!chat.running());
}

#[test]
fn a_draft_shows_each_new_part_once_and_the_final_text_adds_only_the_rest() {
    let mut chat = ChatState::default();
    let draft = |part, text: &str, snapshot| v2::Draft {
        chat_id: "c".into(),
        run_id: "r".into(),
        part,
        text: text.into(),
        snapshot,
    };
    assert_eq!(chunks(&chat.draft(&draft(1, "Hel", false))), ["Hel"]);
    assert!(chat.draft(&draft(3, "skipped", false)).is_empty());
    assert_eq!(chunks(&chat.draft(&draft(2, "lo", false))), ["lo"]);
    assert_eq!(chunks(&chat.draft(&draft(2, "Hello wo", true))), [" wo"]);
    assert_eq!(chunks(&chat.item(&text(1, "r", "Hello world"))), ["rld"]);
}

#[test]
fn a_replayed_question_is_not_asked_but_stays_open_until_resolved() {
    let mut chat = ChatState::default();
    let call = item(
        1,
        "r",
        Body::ToolCall(v2::ToolCall {
            call_id: "c1".into(),
            tool: "server.run".into(),
            title: "prod · reboot".into(),
            ..Default::default()
        }),
    );
    let ask = item(
        2,
        "r",
        Body::ApprovalRequest(v2::ApprovalRequest {
            approval_id: "a1".into(),
            call_id: "c1".into(),
        }),
    );
    assert_eq!(chat.replay(&call).len(), 1);
    assert!(chat.replay(&ask).is_empty());
    let open = chat.open_approvals();
    assert!(matches!(
        open.as_slice(),
        [Output::Permission { approval, title, .. }] if approval == "a1" && title == "prod · reboot"
    ));
    chat.item(&item(
        3,
        "r",
        Body::ApprovalResolution(v2::ApprovalResolution {
            approval_id: "a1".into(),
            ..Default::default()
        }),
    ));
    assert!(chat.open_approvals().is_empty());
    assert_eq!(chat.last_seq(), 3);
    assert!(
        chat.item(&ask).is_empty(),
        "an old item is not applied twice"
    );
}
