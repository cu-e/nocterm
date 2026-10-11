//! Output-only session recording presentation state.
use gpui_kit::Context;
use nocterm_session::recording::RecordingOptions;
use nocterm_store::{AppendHandle, RecordingRegistry};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecordingStatus {
    path: Option<PathBuf>,
    error: Option<String>,
    finished: bool,
    bytes: u64,
}

impl RecordingStatus {
    pub fn path(&self) -> Option<PathBuf> {
        self.path.clone()
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn finished(&self) -> bool {
        self.finished
    }
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

pub(crate) struct OutputRecording {
    options: nocterm_session::LoggingOptions,
    registry: Option<Arc<RecordingRegistry>>,
    current: Option<AppendHandle>,
    last_status: Arc<Mutex<RecordingStatus>>,
}

impl Default for OutputRecording {
    fn default() -> Self {
        Self {
            options: Default::default(),
            registry: None,
            current: None,
            last_status: Arc::new(Mutex::new(RecordingStatus::default())),
        }
    }
}

impl OutputRecording {
    pub(crate) fn set_registry(&mut self, registry: Arc<RecordingRegistry>) {
        self.registry = Some(registry);
    }

    pub(crate) fn set_options(&mut self, options: nocterm_session::LoggingOptions) {
        self.options = options;
    }

    pub(crate) fn options(&self) -> &nocterm_session::LoggingOptions {
        &self.options
    }

    pub(crate) fn is_active(&self) -> bool {
        self.current.is_some()
    }

    pub(crate) fn status(&self) -> Option<RecordingStatus> {
        let last = self
            .last_status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(current) = &self.current {
            let s = current.status();
            Some(RecordingStatus {
                path: s.path().cloned().or(last.path),
                error: s.error().map(|e| e.to_string()).or(last.error),
                finished: s.finished(),
                bytes: s.bytes().max(last.bytes),
            })
        } else if last.path.is_some() || last.bytes > 0 || last.finished {
            Some(last)
        } else {
            None
        }
    }

    pub(crate) fn start(
        &mut self,
        cx: &mut Context<crate::terminal::Terminal>,
    ) -> Result<(), String> {
        if self.is_active() {
            return Ok(());
        }
        let Some(registry) = &self.registry else {
            return Err("Recording registry is unavailable".into());
        };

        let rec_options = RecordingOptions {
            directory: self.options.directory.clone(),
            max_file_mib: self.options.max_file_mib,
        };

        let handle = registry.open(rec_options, cx);
        self.current = Some(handle);
        cx.notify();
        Ok(())
    }

    pub(crate) fn output(&mut self, bytes: &[u8], cx: &mut Context<crate::terminal::Terminal>) {
        if let Some(current) = &self.current {
            current.output(bytes, cx);
        }
    }

    pub(crate) fn stop(&mut self, cx: &mut Context<crate::terminal::Terminal>) {
        if let Some(current) = self.current.take() {
            current.stop(cx);
            let s = current.status();
            *self.last_status.lock().unwrap_or_else(|e| e.into_inner()) = RecordingStatus {
                path: s.path().cloned(),
                error: s.error().map(|e| e.to_string()),
                finished: true,
                bytes: s.bytes(),
            };
            cx.notify();
        }
    }
}
