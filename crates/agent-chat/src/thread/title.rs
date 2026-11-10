//! What a chat is called in the history.
use nocterm_ai::model::content::Content;
use nocterm_ai::thread::ThreadState;

/// A chat's title: `name`, else the agent's title, else the start of the
/// first message. A chat with nothing in it is a new one.
pub(crate) fn chat_title(name: Option<&str>, state: &ThreadState) -> String {
    if let Some(name) = name.filter(|name| !name.trim().is_empty()) {
        return name.to_owned();
    }
    if state.entries.is_empty() {
        return "New Agent".into();
    }
    state
        .title
        .as_ref()
        .filter(|title| !title.trim().is_empty())
        .cloned()
        .or_else(|| {
            state.entries.iter().find_map(|entry| match entry {
                nocterm_ai::thread::Entry::User(parts) => {
                    parts.iter().find_map(|part| match part {
                        Content::Text(text) if !text.text.trim().is_empty() => {
                            Some(text.text.trim().chars().take(60).collect())
                        }
                        _ => None,
                    })
                }
                _ => None,
            })
        })
        .unwrap_or_else(|| "New Agent".into())
}
