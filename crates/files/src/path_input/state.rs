//! Location, editor navigation and completion each own their transitions.
use super::{Candidate, Cycle, Directory, NavigationMode, Query};
use gpui_kit::Task;
use nocterm_session::{
    RemoteFs,
    local_fs::{CancelFlag, LocalFs},
};
use std::sync::Arc;

pub(super) struct LocationState {
    directory: Option<Directory>,
    home: Option<Directory>,
    remote: Option<Arc<dyn RemoteFs>>,
    local: Arc<dyn LocalFs>,
}
impl LocationState {
    pub(super) fn new(local: Arc<dyn LocalFs>) -> Self {
        Self {
            directory: None,
            home: None,
            remote: None,
            local,
        }
    }
    pub(super) fn reset_remote(&mut self, remote: Option<Arc<dyn RemoteFs>>) {
        self.directory = None;
        self.home = None;
        self.remote = remote;
    }
    pub(super) fn accept(&mut self, directory: Directory, home: Option<Directory>) {
        self.directory = Some(directory);
        self.home = home;
    }
    pub(super) fn directory(&self) -> Option<&Directory> {
        self.directory.as_ref()
    }
    pub(super) fn home(&self) -> Option<&Directory> {
        self.home.as_ref()
    }
    pub(super) fn ports(&self) -> (Arc<dyn LocalFs>, Option<Arc<dyn RemoteFs>>) {
        (self.local.clone(), self.remote.clone())
    }
}

#[derive(Default)]
pub(super) struct NavigationState {
    editing: bool,
    navigating: bool,
    mode: NavigationMode,
    observed: String,
    error: Option<String>,
    encoding_error: Option<String>,
}
impl NavigationState {
    pub(super) fn editing(&self) -> bool {
        self.editing
    }
    pub(super) fn navigating(&self) -> bool {
        self.navigating
    }
    pub(super) fn draft_changed(&mut self, value: &str) -> bool {
        if value == self.observed {
            return false;
        }
        self.observed = value.to_owned();
        self.error = None;
        true
    }
    pub(super) fn replaced(&mut self, value: &str) {
        self.observed = value.to_owned();
    }
    pub(super) fn reset(&mut self) {
        self.editing = false;
        self.navigating = false;
        self.mode = NavigationMode::CloseEditor;
        self.error = None;
        self.encoding_error = None;
    }
    pub(super) fn begin(&mut self, mode: NavigationMode) {
        self.mode = mode;
        self.navigating = true;
        self.error = None;
        self.encoding_error = None;
    }
    pub(super) fn accepted(&mut self, value: Option<&str>) {
        self.editing = self.editing && self.mode == NavigationMode::KeepEditing && value.is_some();
        self.navigating = false;
        self.mode = NavigationMode::CloseEditor;
        self.error = None;
        self.encoding_error = None;
        if self.editing {
            self.replaced(value.unwrap());
        }
    }
    pub(super) fn edit(&mut self, value: &str) {
        self.editing = true;
        self.error = None;
        self.encoding_error = None;
        self.replaced(value);
    }
    pub(super) fn close_editor(&mut self) {
        self.editing = false;
        self.mode = NavigationMode::CloseEditor;
        self.error = None;
    }
    pub(super) fn failed(&mut self, error: String) {
        self.navigating = false;
        self.mode = NavigationMode::CloseEditor;
        self.error = Some(error);
    }
    pub(super) fn report_error(&mut self, error: String) {
        self.error = Some(error);
    }
    pub(super) fn clear_error(&mut self) {
        self.error = None;
    }
    pub(super) fn unrepresentable(&mut self) {
        self.encoding_error = Some("This directory path cannot be represented as text.".into());
    }
    pub(super) fn visible_error(&self) -> Option<String> {
        self.encoding_error
            .clone()
            .or_else(|| self.error.clone().filter(|_| self.editing))
    }
}

#[derive(Default)]
pub(super) struct CompletionState {
    generation: u64,
    cycle: Option<Cycle>,
    cache: Option<(Directory, Vec<Candidate>)>,
    pending: Option<(Query, i64)>,
    cancel: CancelFlag,
    task: Option<Task<()>>,
}
impl CompletionState {
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }
    pub(super) fn cycle(&self) -> Option<&Cycle> {
        self.cycle.as_ref()
    }
    pub(super) fn loading(&self) -> bool {
        self.pending.is_some()
    }
    pub(super) fn visible(&self) -> bool {
        self.loading() || self.cycle.is_some()
    }
    pub(super) fn invalidate(&mut self, cache: bool) {
        self.generation = self.generation.wrapping_add(1);
        self.cancel.cancel();
        self.task = None;
        self.pending = None;
        self.cycle = None;
        if cache {
            self.cache = None;
        }
    }
    pub(super) fn continue_cycle(&mut self) -> bool {
        if let Some(cycle) = &self.cycle {
            if cycle.candidate_count() != 1 || !cycle.candidate(0).directory {
                return true;
            }
            self.cycle = None;
        }
        false
    }
    pub(super) fn adjust_pending(&mut self, reverse: bool) -> bool {
        let Some((_, position)) = &mut self.pending else {
            return false;
        };
        *position = position.saturating_add(if reverse { -1 } else { 1 });
        true
    }
    pub(super) fn cached(&mut self, query: Query) -> Result<(), Query> {
        if let Some((directory, entries)) = &self.cache
            && directory == &query.directory
        {
            self.cycle = Some(Cycle::new(query, entries));
            Ok(())
        } else {
            Err(query)
        }
    }
    pub(super) fn begin(&mut self, query: Query, reverse: bool) -> (u64, CancelFlag) {
        self.generation = self.generation.wrapping_add(1);
        self.cancel = CancelFlag::default();
        self.pending = Some((query, if reverse { -1 } else { 0 }));
        (self.generation, self.cancel.clone())
    }
    pub(super) fn own_task(&mut self, task: Task<()>) {
        self.task = Some(task);
    }
    pub(super) fn loaded(&mut self, entries: Vec<Candidate>) -> Option<String> {
        let (query, position) = self.pending.take()?;
        self.cache = Some((query.directory.clone(), entries.clone()));
        let mut cycle = Cycle::new(query, &entries);
        let value = if cycle.candidate_count() == 0 {
            None
        } else {
            cycle.choose(position.rem_euclid(cycle.candidate_count() as i64) as usize)
        };
        self.cycle = Some(cycle);
        value
    }
    pub(super) fn failed(&mut self) {
        self.pending = None;
    }
    pub(super) fn advance(&mut self, reverse: bool) -> Option<String> {
        self.cycle.as_mut().and_then(|cycle| cycle.step(reverse))
    }
    pub(super) fn choose(&mut self, index: usize) -> Option<String> {
        self.cycle.as_mut().and_then(|cycle| cycle.choose(index))
    }
    pub(super) fn selected(&self) -> Option<usize> {
        self.cycle.as_ref().and_then(|cycle| cycle.selected())
    }
}
