use super::*;
use crate::testing::{FakeConversation, row, run};
use crate::{CloudConversation as _, ConversationAction, ConversationUpdate, Freshness};
use futures::StreamExt as _;

fn delta(first_index: u64, changed_from: Option<u64>, rows: Vec<Row>) -> ConversationDelta {
    ConversationDelta {
        first_index,
        changed_from,
        rows,
        ..ConversationDelta::default()
    }
}

fn indexed(index: u64, text: &str) -> Row {
    Row {
        index,
        ..row(RowRole::Assistant, None, text)
    }
}

fn texts(transcript: &CloudTranscript) -> Vec<(u64, String)> {
    transcript
        .rows()
        .iter()
        .map(|row| (row.index, row.text.clone()))
        .collect()
}

#[test]
fn a_delta_appends_replaces_from_its_index_and_drops_rows_below_the_first() {
    let mut transcript = CloudTranscript::new();
    assert!(transcript.apply(delta(0, Some(0), vec![indexed(0, "a"), indexed(1, "b")])));
    assert!(transcript.apply(delta(0, Some(1), vec![indexed(1, "bc"), indexed(2, "d")])));
    assert_eq!(
        texts(&transcript),
        [(0, "a".into()), (1, "bc".into()), (2, "d".into())]
    );
    assert!(transcript.apply(delta(2, None, Vec::new())));
    assert_eq!(texts(&transcript), [(2, "d".into())]);
}

#[test]
fn a_reset_replaces_everything_and_absent_parts_keep_what_is_held() {
    let mut transcript = CloudTranscript::new();
    let command = CommandKey::from_bytes([1; 16]);
    let key = RunKey::from_bytes([2; 16]);
    transcript.apply(ConversationDelta {
        runs: Some(vec![run(key, RunStatus::Running, command)]),
        ..delta(0, Some(0), vec![indexed(0, "old")])
    });
    // Runs absent: the held run stays; only the scalars move.
    assert!(!transcript.apply(ConversationDelta {
        control_revision: 4,
        ..ConversationDelta::default()
    }));
    assert_eq!(transcript.runs().len(), 1);
    assert_eq!(transcript.control_revision(), 4);
    transcript.apply(ConversationDelta {
        reset: true,
        ..delta(5, Some(5), vec![indexed(5, "snapshot")])
    });
    assert_eq!(texts(&transcript), [(5, "snapshot".into())]);
    assert!(transcript.runs().is_empty());
}

#[test]
fn the_shown_run_is_the_latest_unfinished_one_else_the_latest() {
    let mut transcript = CloudTranscript::new();
    let command = CommandKey::from_bytes([1; 16]);
    let (first, second) = (RunKey::from_bytes([1; 16]), RunKey::from_bytes([2; 16]));
    transcript.apply(ConversationDelta {
        runs: Some(vec![
            run(first, RunStatus::Running, command),
            run(second, RunStatus::Completed, command),
        ]),
        ..ConversationDelta::default()
    });
    assert_eq!(transcript.shown_run().map(|run| run.run), Some(first));
    transcript.apply(ConversationDelta {
        runs: Some(vec![run(first, RunStatus::Stopped, command)]),
        ..ConversationDelta::default()
    });
    assert_eq!(transcript.shown_run().map(|run| run.run), Some(first));
    assert!(RunStatus::OutcomeUnknown.finished() && !RunStatus::NeedsGrant.finished());
}

#[test]
fn the_fake_starts_each_subscription_with_the_whole_transcript_and_its_freshness() {
    futures::executor::block_on(async {
        let fake = FakeConversation::new("chat");
        fake.push_rows(vec![row(RowRole::Assistant, None, "hello")]);
        fake.set_freshness(Freshness::Current { witnessed_at_ms: 9 });
        let mut updates = fake.subscribe().unwrap();
        let Some(ConversationUpdate::Transcript(first)) = updates.next().await else {
            panic!("a subscription starts with the transcript");
        };
        assert!(first.reset);
        let mut held = CloudTranscript::new();
        held.apply(first);
        assert_eq!(texts(&held), [(0, "hello".into())]);
        assert!(matches!(
            updates.next().await,
            Some(ConversationUpdate::Freshness(Freshness::Current { .. }))
        ));
    });
}

#[test]
fn a_delivered_prompt_is_pending_until_the_runner_accepts_it() {
    futures::executor::block_on(async {
        let fake = FakeConversation::new("chat");
        let command = fake
            .deliver(ConversationAction::Prompt {
                text: "hi".into(),
                server: None,
                model: None,
                images: Vec::new(),
            })
            .await
            .unwrap();
        let pending = fake.transcript().pending_command(command).cloned().unwrap();
        assert_eq!(
            (pending.kind, pending.state),
            (CommandKind::Prompt, CommandState::Sending)
        );
        fake.accept_prompt(command, RunKey::from_bytes([3; 16]), RunStatus::Queued);
        let transcript = fake.transcript();
        assert_eq!(
            transcript.pending_command(command).map(|known| known.state),
            Some(CommandState::Accepted)
        );
        assert_eq!(texts(&transcript), [(0, "hi".into())]);
        assert!(CommandState::Refused.settled() && !CommandState::Delivered.settled());
    });
}

#[test]
fn an_ended_fake_is_stale_and_refuses_new_requests() {
    futures::executor::block_on(async {
        let fake = FakeConversation::new("chat");
        let ended = fake.ended();
        fake.end();
        ended.await;
        assert!(matches!(
            fake.check_current(),
            Err(crate::CloudConversationError::Stale)
        ));
        assert!(fake.subscribe().is_err());
        assert!(
            fake.deliver(ConversationAction::Stop { run: None })
                .await
                .is_err()
        );
    });
}

#[test]
fn a_binding_replaces_the_held_one_only_when_present_and_lists_its_selectable_servers() {
    use crate::{BoundServer, ServerAccess, ServerBinding, ServerKey};
    let server = |byte: u8, selectable: bool| BoundServer {
        server: ServerKey::from_bytes([byte; 16]),
        host: format!("h{byte}"),
        port: 22,
        user: "deploy".into(),
        access: if selectable {
            ServerAccess::Active { until_ms: 9 }
        } else {
            ServerAccess::Unreachable
        },
        selectable,
    };
    let mut transcript = CloudTranscript::new();
    assert!(!transcript.servers().is_bound());
    transcript.apply(ConversationDelta {
        servers: Some(ServerBinding {
            servers: vec![server(1, true), server(2, false)],
            expires_at_ms: Some(50),
        }),
        ..ConversationDelta::default()
    });
    transcript.apply(ConversationDelta::default());
    let binding = transcript.servers();
    assert!(binding.is_bound());
    assert_eq!(
        binding
            .selectable()
            .map(|known| known.host.as_str())
            .collect::<Vec<_>>(),
        ["h1"]
    );
    transcript.apply(ConversationDelta {
        servers: Some(ServerBinding::default()),
        ..ConversationDelta::default()
    });
    assert!(!transcript.servers().is_bound());
}

#[test]
fn the_fake_records_bindings_and_refuses_an_empty_choice_or_a_long_window() {
    use crate::{CloudConversationError, ServerKey, ServerRefusal};
    futures::executor::block_on(async {
        let fake = FakeConversation::new("chat");
        let day = std::time::Duration::from_secs(86_400);
        let key = ServerKey::from_bytes([3; 16]);
        fake.bind_servers(vec![key], 7 * day).await.unwrap();
        assert!(matches!(
            fake.bind_servers(Vec::new(), day).await,
            Err(CloudConversationError::Servers(ServerRefusal::Servers))
        ));
        assert!(matches!(
            fake.bind_servers(vec![key], 31 * day).await,
            Err(CloudConversationError::Servers(ServerRefusal::Window))
        ));
        fake.unbind_servers().await.unwrap();
        assert_eq!(fake.bindings(), [(vec![key], 7 * day)]);
        assert_eq!(fake.unbinds(), 1);
    });
}
