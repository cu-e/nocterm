//! Original transcript behavior through admitted original native capabilities.
use crate::model::retention::document::tests::{text, user_text};
use crate::{
    history::{
        HistoryOrigin, NativeMaterialized,
        live::{LiveEpoch, LiveRouted},
        native::NativeChat,
        physical::{ContentSlot, RowBody},
    },
    model::{
        content::MediaKind,
        ids::SessionId,
        json::deferred::{DecodeLedger, literal::LiteralBuilder},
        native::{NativeList, NativeOwned},
        notice::SessionUpdate,
        retention::RetentionAuthority,
        tools::{ToolKind, ToolStatus},
    },
    session::SessionState,
    thread::{ThreadChange, ThreadState},
};
struct Fixture {
    state: ThreadState,
    context: crate::history::source::SourceContext,
    _lease: crate::history::source::SourceLease,
    epoch: LiveEpoch,
}
impl Fixture {
    fn new() -> Self {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let mut state = ThreadState::new(
            &authority,
            "agent",
            "f2f8f6ac-eae5-4dd7-b4bc-3713da3ed73c",
            0,
        )
        .unwrap();
        let lease = state.source().capture().unwrap();
        let context = lease.context().unwrap();
        let epoch = LiveEpoch::new(
            SessionState::default().ticket(),
            1,
            SessionId::new("original"),
        );
        state.live_mut().accept_epoch(epoch.clone());
        Self {
            state,
            context,
            _lease: lease,
            epoch,
        }
    }
    fn ledger(&self) -> &DecodeLedger {
        self.context.ledger()
    }
    fn apply(&mut self, update: NativeOwned<SessionUpdate>, now: u64) -> ThreadChange {
        let admission = self.state.live().admission(&self.epoch).unwrap();
        match self
            .state
            .live_mut()
            .apply_owned(admission, update, now)
            .unwrap()
        {
            LiveRouted::Applied(applied) => applied.change(),
            LiveRouted::Forward(_) => panic!("original notice was not applied"),
        }
    }
    fn user(&mut self, value: Option<&str>, now: u64) {
        let mut content = NativeList::new(self.ledger(), usize::from(value.is_some())).unwrap();
        if let Some(value) = value {
            content.push(user_text(self.ledger(), value)).unwrap();
        }
        let content = content.finish();
        let admission = self.state.live().local_admission();
        self.state
            .live_mut()
            .append_user(admission, content, now)
            .unwrap();
    }
    fn restore(&mut self, times: Vec<u64>) {
        let delivery = self.state.reader().capture().unwrap().read(|ledger| {
            let chat = crate::history::SavedChat::from_parts(
                ledger.authority(),
                "f2f8f6ac-eae5-4dd7-b4bc-3713da3ed73c".into(),
                crate::history::ChatMetadata {
                    agent_id: "agent".into(),
                    title: None,
                    name: None,
                    pinned: false,
                    updated: 0,
                    model: None,
                },
                crate::history::ResumeDescriptor::default(),
                crate::history::TranscriptData {
                    entries: vec![crate::thread::Entry::Agent("old".into())],
                    times,
                },
                crate::history::ComposerData::default(),
            )
            .unwrap();
            Ok(NativeMaterialized::new(
                NativeChat::from_fixture(chat, ledger).unwrap(),
                HistoryOrigin::Version2,
                None,
                0,
            ))
        });
        self.state.live_mut().install_delivery(delivery).unwrap();
    }
}
#[test]
fn reducer_merges_chunks_and_keeps_message_boundaries() {
    let mut f = Fixture::new();
    for value in ["hello ", "world"] {
        f.apply(
            crate::model::native::notice::agent_chunk(user_text(f.ledger(), value)),
            1,
        );
    }
    assert!(
        matches!(f.state.rows()[0].view(), RowBody::Agent(value) if &*value.read() == "hello world")
    );
    f.user(Some("next"), 1);
    f.apply(
        crate::model::native::notice::thought_chunk(user_text(f.ledger(), "thinking")),
        1,
    );
    f.apply(
        crate::model::native::notice::agent_chunk(user_text(f.ledger(), "response")),
        1,
    );
    assert_eq!(f.state.rows().len(), 4);
}
fn started_read(ledger: &DecodeLedger) -> NativeOwned<SessionUpdate> {
    let header = crate::model::native::tools::header(
        crate::model::native::tools::tool_id(text(ledger, "1")).unwrap(),
        text(ledger, "read"),
        NativeOwned::absent(ledger).unwrap(),
        ToolKind::Read,
        ToolStatus::Pending,
    )
    .unwrap();
    let record = crate::model::native::tools::record(
        header,
        NativeList::new(ledger, 0).unwrap().finish(),
        NativeList::new(ledger, 0).unwrap().finish(),
        NativeOwned::absent(ledger).unwrap(),
        NativeOwned::absent(ledger).unwrap(),
        NativeOwned::absent(ledger).unwrap(),
        NativeOwned::absent(ledger).unwrap(),
    )
    .unwrap();
    crate::model::native::notice::tool_started(record)
}
fn succeeded_read(ledger: &DecodeLedger) -> NativeOwned<SessionUpdate> {
    let mut json = LiteralBuilder::new(ledger).unwrap();
    json.start_object().unwrap();
    let mut key = json.text().unwrap();
    key.append("text").unwrap();
    json.key(key).unwrap();
    let mut value = json.text().unwrap();
    value.append("ok").unwrap();
    json.string(value).unwrap();
    json.end_container().unwrap();
    let result =
        crate::model::native::content::json(json.finish_native().unwrap(), ledger).unwrap();
    let patch = crate::model::native::patch::with_result(
        crate::model::native::patch::with_title(
            crate::model::native::patch::tool_patch(
                crate::model::native::tools::tool_id(text(ledger, "1")).unwrap(),
                None,
                Some(ToolStatus::Succeeded),
            ),
            text(ledger, "done"),
        )
        .unwrap(),
        result,
    )
    .unwrap();
    crate::model::native::notice::tool_changed(patch)
}
#[test]
fn reducer_handles_transcript_metadata_usage_and_tool_replacement() {
    let mut f = Fixture::new();
    assert_ne!(f.apply(started_read(f.ledger()), 1), ThreadChange::Ignored);
    assert_ne!(
        f.apply(succeeded_read(f.ledger()), 1),
        ThreadChange::Ignored
    );
    let plan = NativeList::new(f.ledger(), 0).unwrap().finish();
    assert_ne!(
        f.apply(crate::model::native::notice::plan(plan), 1),
        ThreadChange::Ignored
    );
    let title = crate::model::native::notice::from_presence(text(f.ledger(), "A title").presence());
    assert_ne!(
        f.apply(crate::model::native::notice::info(title), 1),
        ThreadChange::Ignored
    );
    let context =
        crate::model::native::usage::context(50, 100, NativeOwned::absent(f.ledger()).unwrap());
    let usage = crate::model::native::notice::usage_notice(
        context,
        NativeList::new(f.ledger(), 0).unwrap().finish(),
    )
    .unwrap();
    assert_ne!(
        f.apply(crate::model::native::notice::usage(usage), 1),
        ThreadChange::Ignored
    );
    assert_ne!(
        f.apply(
            crate::model::native::notice::user_chunk(user_text(f.ledger(), "user")),
            1
        ),
        ThreadChange::Ignored
    );
    assert!(
        matches!(f.state.rows()[0].view(), RowBody::Tool(value) if &*value.title() == "done" && value.status() == ToolStatus::Succeeded)
    );
    assert_eq!(f.state.title().as_deref(), Some("A title"));
    assert_eq!(f.state.usage().unwrap().used, 50);
    let clear = NativeOwned::<String>::null(f.ledger()).unwrap();
    f.apply(
        crate::model::native::notice::info(crate::model::native::notice::from_presence(clear)),
        1,
    );
    assert!(f.state.title().is_none());
}
#[test]
fn image_chunks_are_retained() {
    let mut f = Fixture::new();
    let media = crate::model::native::content::media(
        MediaKind::Image,
        text(f.ledger(), "AA=="),
        text(f.ledger(), "image/png"),
        NativeOwned::absent(f.ledger()).unwrap(),
        NativeOwned::absent(f.ledger()).unwrap(),
        NativeOwned::absent(f.ledger()).unwrap(),
    )
    .unwrap();
    f.apply(
        crate::model::native::notice::agent_chunk(crate::model::native::content::from_media(media)),
        1,
    );
    assert!(
        matches!(f.state.rows().last().unwrap().view(), RowBody::Content(ContentSlot::Media(media)) if media.kind() == MediaKind::Image)
    );
}
#[test]
fn reducer_uses_supplied_timestamps_and_preserves_the_first_chunk_time() {
    let mut f = Fixture::new();
    f.apply(
        crate::model::native::notice::agent_chunk(user_text(f.ledger(), "first")),
        10,
    );
    f.apply(
        crate::model::native::notice::agent_chunk(user_text(f.ledger(), "second")),
        20,
    );
    f.user(Some("next"), 30);
    assert_eq!(f.state.time(0), Some(10));
    assert_eq!(f.state.time(1), Some(30));
    let title = crate::model::native::notice::from_presence(text(f.ledger(), "Title").presence());
    f.apply(crate::model::native::notice::info(title), 40);
    assert_eq!(f.state.times(), [10, 30]);
}
#[test]
fn restoring_literal_times_is_clean_until_a_real_row_adds_alignment() {
    let mut f = Fixture::new();
    f.restore(Vec::new());
    assert!(f.state.times().is_empty());
    f.user(None, 19);
    assert_eq!(f.state.times(), [0, 19]);
    f.restore(vec![4, 5]);
    assert_eq!(f.state.times(), [4, 5]);
    f.user(None, 20);
    assert_eq!(f.state.times(), [4, 20]);
}
#[test]
fn transcript_search_keeps_contextual_unicode_lowercase() {
    let mut f = Fixture::new();
    f.apply(
        crate::model::native::notice::agent_chunk(user_text(f.ledger(), "ΟΣ")),
        1,
    );
    assert!(f.state.mentions("ος").unwrap());
    assert!(!f.state.mentions("οσ").unwrap());
    assert!(f.state.mentions("").unwrap());
}
