//! Borrowed physical rows supply a fallback title without rebuilding transcript entries.
use nocterm_ai::thread::ThreadState;
pub(super) struct ChatTitle<'a> {
    name: Option<&'a str>,
    state: &'a ThreadState,
}
impl<'a> ChatTitle<'a> {
    pub(super) fn new(name: Option<&'a str>, state: &'a ThreadState) -> Self {
        Self { name, state }
    }
    pub(super) fn display(&self) -> String {
        if let Some(name) = self.name.filter(|name| !name.trim().is_empty()) {
            return name.to_owned();
        }
        if self.state.rows().is_empty() {
            return "New Agent".into();
        }
        self.state
            .title()
            .filter(|title| !title.trim().is_empty())
            .map(|title| title.to_string())
            .or_else(|| {
                self.state
                    .rows()
                    .iter()
                    .find_map(|entry| match entry.view() {
                        nocterm_ai::history::physical::RowBody::User(parts) => {
                            parts.contents().find_map(|part| match part {
                                nocterm_ai::history::physical::ContentSlot::Text(text)
                                    if !text.text().trim().is_empty() =>
                                {
                                    Some(text.text().trim().chars().take(60).collect())
                                }
                                _ => None,
                            })
                        }
                        _ => None,
                    })
            })
            .unwrap_or_else(|| "New Agent".into())
    }
}
