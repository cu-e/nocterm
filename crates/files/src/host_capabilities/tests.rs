//! The counts follow the transfer queue.
use std::sync::Arc;

use futures::executor::block_on;
use gpui_kit::TestAppContext;
use nocterm_plugin_api::{
    HostError,
    capability::transfers::{Counts, CountsNow},
};
use nocterm_plugin_model::PluginId;
use nocterm_plugin_ports::Observe;
use nocterm_session::{HostAddress, RemoteFs as _, Target, testing::InMemoryFs};
use nocterm_transfers::{
    CollisionPolicy, DownloadRequest, TransferCause, TransferEvent, TransferId, TransferIssue,
    TransferOperation, TransferService, testing::FakeTransfers,
};

use super::TransferCounts;

struct Queue {
    recording: Arc<FakeTransfers>,
    counts: TransferCounts,
    fs: Arc<InMemoryFs>,
}

fn queue() -> Queue {
    let recording = Arc::new(FakeTransfers::default());
    let fs = Arc::new(InMemoryFs::default());
    block_on(fs.create_dir("/home")).unwrap();
    Queue {
        counts: TransferCounts::over(recording.clone()),
        recording,
        fs,
    }
}

impl Queue {
    fn start(&self) -> TransferId {
        self.recording
            .enqueue_download(DownloadRequest {
                sources: vec!["/home".into()],
                host: HostAddress::direct(Target::new("test", "host", 22)),
                local_destination: "/tmp/received".into(),
                fs: self.fs.clone(),
                collisions: CollisionPolicy::Skip,
            })
            .unwrap()
    }

    fn finish(&self, id: TransferId) {
        for event in [
            TransferEvent::DiscoveryComplete,
            TransferEvent::WorkersDrained,
        ] {
            self.recording.advance(id, event).unwrap();
        }
    }

    fn fail_a_file(&self, id: TransferId) {
        let issue = TransferIssue::new(
            None,
            TransferOperation::Discover,
            TransferCause::InvalidName,
        );
        self.recording
            .advance(id, TransferEvent::FileFailed(issue))
            .unwrap();
    }

    fn read(&self, cx: &mut TestAppContext) -> Result<Counts, HostError> {
        let plugin = PluginId::user("acme", "queue").unwrap();
        cx.update(|cx| Observe::<CountsNow>::sample(&self.counts, &plugin, cx))
    }
}

fn counts(active: u16, failed: u16) -> Result<Counts, HostError> {
    Ok(Counts { active, failed })
}

#[gpui_kit::test]
fn an_empty_queue_has_nothing_running_or_failed(cx: &mut TestAppContext) {
    assert_eq!(queue().read(cx), counts(0, 0));
}

#[gpui_kit::test]
fn running_transfers_are_counted_until_they_finish(cx: &mut TestAppContext) {
    let queue = queue();
    let first = queue.start();
    queue.start();
    assert_eq!(queue.read(cx), counts(2, 0));
    queue.finish(first);
    assert_eq!(queue.read(cx), counts(1, 0), "a completed transfer is done");
}

#[gpui_kit::test]
fn a_transfer_with_a_failed_file_counts_as_failed_once_finished(cx: &mut TestAppContext) {
    let queue = queue();
    let id = queue.start();
    queue.fail_a_file(id);
    assert_eq!(
        queue.read(cx),
        counts(1, 1),
        "the failure shows while it runs"
    );
    queue.finish(id);
    assert_eq!(queue.read(cx), counts(0, 1));
}

#[gpui_kit::test]
fn a_cancelled_transfer_is_neither_running_nor_failed(cx: &mut TestAppContext) {
    let queue = queue();
    let id = queue.start();
    queue.recording.cancel(id).unwrap();
    assert_eq!(queue.read(cx), counts(0, 0));
}

#[gpui_kit::test]
fn reading_the_counts_changes_nothing_in_the_queue(cx: &mut TestAppContext) {
    let queue = queue();
    queue.start();
    let before = queue.recording.snapshot();
    for _ in 0..3 {
        queue.read(cx).unwrap();
    }
    assert_eq!(queue.recording.snapshot(), before);
}
