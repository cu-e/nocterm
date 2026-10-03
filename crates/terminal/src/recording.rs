//! Bounded output-only capture. File I/O never runs on the UI executor.
use std::{
    fs::{self, OpenOptions},
    io::Write as _,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{self, SyncSender},
    },
    thread,
};
const CHUNK: usize = 8192;
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecordingStatus {
    pub path: Option<PathBuf>,
    pub error: Option<String>,
    pub finished: bool,
    pub bytes: u64,
}
struct Control {
    sender: Mutex<Option<SyncSender<Vec<u8>>>>,
    state: Arc<Mutex<RecordingStatus>>,
}
pub(crate) struct Recorder {
    control: Arc<Control>,
}
#[derive(Default)]
pub(crate) struct Registry(Mutex<Vec<Arc<Control>>>);
impl Registry {
    pub(crate) fn track(&self, recorder: &Recorder) {
        let mut controls = self.0.lock().unwrap_or_else(|e| e.into_inner());
        controls.retain(|control| {
            !control
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .finished
        });
        controls.push(recorder.control.clone());
    }
    pub(crate) fn stop_all(&self) {
        for control in self.0.lock().unwrap_or_else(|e| e.into_inner()).iter() {
            *control.sender.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }
    pub(crate) fn finished(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .all(|control| {
                control
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .finished
            })
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop();
    }
}
impl Recorder {
    pub(crate) fn start(directory: PathBuf, max_file_mib: u32) -> Result<Self, String> {
        let id = nocterm_session::CredentialId::generate()
            .map_err(|_| "Could not generate a unique log filename.")?;
        let path = directory.join(format!("session-{id}.log"));
        let state = Arc::new(Mutex::new(RecordingStatus {
            path: Some(path.clone()),
            ..Default::default()
        }));
        let worker_state = state.clone();
        let (sender, receiver) = mpsc::sync_channel::<Vec<u8>>(128);
        thread::Builder::new()
            .name("nocterm-output-log".into())
            .spawn(move || {
                let write = || -> Result<(), String> {
                    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
                    let mut options = OpenOptions::new();
                    options.write(true).create_new(true);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::OpenOptionsExt as _;
                        options.mode(0o600);
                    }
                    let mut file = options.open(&path).map_err(|error| error.to_string())?;
                    let limit = u64::from(max_file_mib.clamp(1, 1024)) * 1024 * 1024;
                    let mut written = 0;
                    for bytes in receiver {
                        if written + bytes.len() as u64 > limit {
                            file.flush().map_err(|error| error.to_string())?;
                            return Err("Log size limit reached. Recording stopped.".into());
                        }
                        file.write_all(&bytes).map_err(|error| error.to_string())?;
                        written += bytes.len() as u64;
                        worker_state.lock().unwrap_or_else(|e| e.into_inner()).bytes = written;
                    }
                    file.flush()
                        .and_then(|()| file.sync_all())
                        .map_err(|error| error.to_string())
                };
                let result = write();
                let mut state = worker_state.lock().unwrap_or_else(|e| e.into_inner());
                if let Err(error) = result {
                    state.error = Some(format!("Could not record session output: {error}"));
                }
                state.finished = true;
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            control: Arc::new(Control {
                sender: Mutex::new(Some(sender)),
                state,
            }),
        })
    }
    pub(crate) fn output(&mut self, bytes: &[u8]) {
        if self.status().finished {
            *self
                .control
                .sender
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = None;
            return;
        }
        let mut start = 0;
        while start < bytes.len() {
            let mut end = (start + CHUNK).min(bytes.len());
            while end < bytes.len() && end > start && bytes[end] & 0xc0 == 0x80 {
                end -= 1;
            }
            if end == start {
                end = (start + CHUNK).min(bytes.len());
            }
            let chunk = &bytes[start..end];
            start = end;
            let mut sender = self
                .control
                .sender
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let Some(channel) = sender.as_ref() else {
                break;
            };
            if channel.try_send(chunk.to_vec()).is_err() {
                self.control.state.lock().unwrap_or_else(|e|e.into_inner()).error=Some("Recording stopped because its output queue could not keep up. The log is incomplete.".into());
                *sender = None;
                break;
            }
        }
    }
    pub(crate) fn stop(&mut self) {
        *self
            .control
            .sender
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
    }
    pub(crate) fn active(&self) -> bool {
        self.control
            .sender
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
            && !self.status().finished
    }
    pub(crate) fn status(&self) -> RecordingStatus {
        self.control
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn finish(recorder: &Recorder) -> RecordingStatus {
        for _ in 0..200 {
            let status = recorder.status();
            if status.finished {
                return status;
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("recording worker did not finish")
    }
    #[test]
    fn registry_shutdown_stops_open_recorders_and_drains_before_completion() {
        let directory = tempfile::tempdir().unwrap();
        let registry = Registry::default();
        let mut recorder = Recorder::start(directory.path().into(), 1).unwrap();
        registry.track(&recorder);
        recorder.output("最后 output\n".as_bytes());
        registry.stop_all();
        let status = finish(&recorder);
        assert!(registry.finished());
        assert!(!recorder.active());
        assert_eq!(
            fs::read(status.path.unwrap()).unwrap(),
            "最后 output\n".as_bytes()
        );
    }
    #[test]
    fn full_queue_stops_recording_and_marks_the_log_incomplete() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut recorder = Recorder {
            control: Arc::new(Control {
                sender: Mutex::new(Some(sender)),
                state: Arc::new(Mutex::new(RecordingStatus::default())),
            }),
        };
        recorder.output(b"first");
        recorder.output(b"not accepted");
        assert!(!recorder.active());
        assert!(recorder.status().error.unwrap().contains("incomplete"));
        assert_eq!(receiver.recv().unwrap(), b"first");
        assert!(receiver.recv().is_err());
    }
    #[test]
    fn capture_is_private_unique_and_drains_final_output() {
        let directory = tempfile::tempdir().unwrap();
        let mut recorder = Recorder::start(directory.path().into(), 1).unwrap();
        recorder.output(b"first\nlast output\n");
        recorder.stop();
        let status = finish(&recorder);
        assert!(status.error.is_none());
        assert_eq!(
            fs::read(status.path.as_ref().unwrap()).unwrap(),
            b"first\nlast output\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(status.path.as_ref().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        let mut other = Recorder::start(directory.path().into(), 1).unwrap();
        other.stop();
        assert_ne!(finish(&other).path, status.path);
    }
    #[test]
    fn failed_creation_does_not_overwrite_existing_paths() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("existing");
        fs::write(&path, b"preserve").unwrap();
        let mut recorder = Recorder::start(path.clone(), 1).unwrap();
        recorder.stop();
        assert!(finish(&recorder).error.is_some());
        assert_eq!(fs::read(path).unwrap(), b"preserve");
    }
    #[test]
    fn size_cap_stops_output_with_visible_failure() {
        let directory = tempfile::tempdir().unwrap();
        let mut recorder = Recorder::start(directory.path().into(), 1).unwrap();
        for _ in 0..130 {
            if recorder.status().finished {
                break;
            }
            recorder.output(&vec![b'x'; CHUNK]);
            thread::sleep(std::time::Duration::from_millis(1));
        }
        recorder.stop();
        let status = finish(&recorder);
        assert!(status.error.unwrap().contains("size limit"));
        assert!(fs::metadata(status.path.unwrap()).unwrap().len() <= 1024 * 1024);
    }
}
