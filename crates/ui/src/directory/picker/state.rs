//! Original-source read generations and accepted directory validation.
use gpui_kit::Task;
use nocterm_explorer::directory::{Directory, DirectoryError, DirectoryRead};
use nocterm_session::local_fs::CancelFlag;
use std::sync::Arc;
pub(super) struct PickerState {
    reader: Arc<dyn DirectoryRead>,
    generation: u64,
    cancel: CancelFlag,
    task: Option<Task<()>>,
    accepted: Option<Directory>,
    loading: bool,
    error: Option<DirectoryError>,
}
impl PickerState {
    pub(super) fn new(reader: Arc<dyn DirectoryRead>) -> Self {
        Self {
            reader,
            generation: 0,
            cancel: CancelFlag::default(),
            task: None,
            accepted: None,
            loading: false,
            error: None,
        }
    }
    pub(super) fn replace_reader(&mut self, reader: Arc<dyn DirectoryRead>) {
        self.invalidate();
        self.reader = reader;
    }
    pub(super) fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.cancel.cancel();
        self.task = None;
        self.accepted = None;
        self.loading = false;
        self.error = None;
    }
    pub(super) fn begin(&mut self) -> (u64, CancelFlag, Arc<dyn DirectoryRead>) {
        self.invalidate();
        self.cancel = CancelFlag::default();
        self.loading = true;
        (self.generation, self.cancel.clone(), self.reader.clone())
    }
    pub(super) fn own_task(&mut self, task: Task<()>) {
        self.task = Some(task);
    }
    pub(super) fn is_current(&self, generation: u64) -> bool {
        self.generation == generation && self.loading
    }
    pub(super) fn loading(&self) -> bool {
        self.loading
    }
    pub(super) fn accepted(&self) -> Option<&Directory> {
        self.accepted.as_ref()
    }
    pub(super) fn error(&self) -> Option<&DirectoryError> {
        self.error.as_ref()
    }
    pub(super) fn accept(&mut self, directory: Directory) {
        self.accepted = Some(directory);
        self.loading = false;
        self.error = None;
    }
    pub(super) fn fail(&mut self, error: DirectoryError) {
        self.loading = false;
        self.error = Some(error);
    }
}

impl Drop for PickerState {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
