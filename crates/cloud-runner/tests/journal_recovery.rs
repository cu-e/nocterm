#![cfg(unix)]
mod support;
use futures::executor::block_on;
use nocterm_cloud_model::{identity::JournalSequence, journal::*};
use nocterm_cloud_runner::JournalLimits;
use std::{
    io::{BufRead as _, Write as _},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};
use support::*;

const PROBE_ROOT: &str = "NOCTERM_CLOUD_JOURNAL_PROBE_ROOT";
const PROBE_PAUSE: &str = "NOCTERM_CLOUD_JOURNAL_PROBE_PAUSE";

struct PausingCipher {
    codec: TestCipher,
    calls: AtomicUsize,
}
impl JournalCipher for PausingCipher {
    fn chat_key_epoch(
        &self,
        stream: &nocterm_cloud_model::authority::StreamBinding,
    ) -> Result<nocterm_cloud_model::identity::ChatKeyEpoch, JournalError> {
        self.codec.chat_key_epoch(stream)
    }
    fn seal(&self, context: &JournalCipherContext, plain: &[u8]) -> Result<Vec<u8>, JournalError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == 6 {
            ready("BEFORE_COMMIT");
            loop {
                std::thread::park();
            }
        }
        JournalCipher::seal(&self.codec, context, plain)
    }
    fn open(&self, context: &JournalCipherContext, sealed: &[u8]) -> Result<Vec<u8>, JournalError> {
        JournalCipher::open(&self.codec, context, sealed)
    }
}
fn ready(message: &str) {
    let mut output = std::io::stdout().lock();
    writeln!(output, "{message}").unwrap();
    output.flush().unwrap();
}

#[test]
fn process_worker_commits_an_admission() {
    let temporary = private_root();
    let configured = std::env::var_os(PROBE_ROOT);
    let root = configured
        .as_deref()
        .map_or(temporary.path(), std::path::Path::new);
    let cipher: Arc<dyn JournalCipher> = if std::env::var_os(PROBE_PAUSE).is_some() {
        Arc::new(PausingCipher {
            codec: TestCipher::default(),
            calls: AtomicUsize::new(0),
        })
    } else {
        Arc::new(TestCipher::default())
    };
    let store = open_journal(root, cipher, JournalLimits::default()).unwrap();
    let lease = claim(&store, None);
    commit(
        &store,
        admission(&lease, JournalPosition::default(), acceptance(1, 0)),
    );
    if configured.is_some() {
        ready("AFTER_COMMIT");
        loop {
            std::thread::park();
        }
    }
    assert_eq!(
        block_on(store.read(stream(), JournalSequence::default(), limits()))
            .unwrap()
            .records()
            .len(),
        1
    );
}

struct Probe {
    child: Child,
}
impl Probe {
    fn start(root: &std::path::Path, pause: bool) -> (Self, mpsc::Receiver<String>) {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "process_worker_commits_an_admission",
                "--nocapture",
            ])
            .env(PROBE_ROOT, root)
            .env_remove(PROBE_PAUSE)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if pause {
            command.env(PROBE_PAUSE, "1");
        }
        let mut child = command.spawn().unwrap();
        let output = child.stdout.take().unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(output).lines() {
                let Ok(line) = line else { return };
                if line.contains("BEFORE_COMMIT") || line.contains("AFTER_COMMIT") {
                    let _ = sender.send(line);
                    return;
                }
            }
        });
        (Self { child }, receiver)
    }
    fn kill(&mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn acknowledged_transaction_survives_killing_the_process_without_cleanup() {
    let root = private_root();
    let (mut process, ready) = Probe::start(root.path(), false);
    assert!(
        ready
            .recv_timeout(Duration::from_secs(15))
            .unwrap()
            .contains("AFTER_COMMIT")
    );
    process.kill();
    let store = journal(root.path(), Arc::new(TestCipher::default()));
    let page = block_on(store.read(stream(), JournalSequence::default(), limits())).unwrap();
    assert_eq!(page.records().len(), 1);
    let lease = WriterLease::new(
        stream(),
        nocterm_cloud_model::identity::WriterEpoch::new(1).unwrap(),
    );
    assert!(matches!(
        block_on(store.commit(admission(
            &lease,
            JournalPosition::default(),
            acceptance(1, 0)
        ))),
        Ok(CommitOutcome::Duplicate(_))
    ));
}

#[test]
fn interrupted_transaction_recovers_without_partial_command_checkpoint_or_outbox() {
    let root = private_root();
    let (mut process, ready) = Probe::start(root.path(), true);
    assert!(
        ready
            .recv_timeout(Duration::from_secs(15))
            .unwrap()
            .contains("BEFORE_COMMIT")
    );
    process.kill();
    let store = journal(root.path(), Arc::new(TestCipher::default()));
    let page = block_on(store.read(stream(), JournalSequence::default(), limits())).unwrap();
    assert_eq!(page.head(), JournalPosition::default());
    assert!(page.records().is_empty());
    assert!(block_on(store.checkpoint(stream())).unwrap().is_none());
    assert!(
        block_on(store.outbox(
            stream(),
            nocterm_cloud_model::identity::EventSequence::default(),
            limits()
        ))
        .unwrap()
        .is_empty()
    );
    let lease = WriterLease::new(
        stream(),
        nocterm_cloud_model::identity::WriterEpoch::new(1).unwrap(),
    );
    commit(
        &store,
        admission(&lease, JournalPosition::default(), acceptance(1, 0)),
    );
}
