use crate::model::controls::Change;
#[derive(Clone, Debug, Default)]
pub(super) struct ThreadMetadata {
    title: Option<String>,
}
impl ThreadMetadata {
    pub(super) fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }
    pub(super) fn replace_title(&mut self, title: Option<String>) {
        self.title = title;
    }
    pub(super) fn apply_title(&mut self, title: Change<String>) {
        match title {
            Change::Keep => {}
            Change::Set(value) => self.title = Some(value),
            Change::Clear => self.title = None,
        }
    }
}
