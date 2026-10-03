use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
/// Deliberately contains no authentication, proxy, credential, launch or environment fields.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConnectionDescriptor {
    pub id: String,
    pub name: String,
    pub group: Option<String>,
    pub description: String,
    pub host: String,
    pub port: u16,
    pub user: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TerminalDescriptor {
    pub id: String,
    pub title: String,
    pub local: bool,
    pub connection: Option<ConnectionDescriptor>,
    pub cwd: Option<String>,
    pub status: String,
}
#[derive(Default)]
pub struct OpaqueIds {
    ids: BTreeMap<String, String>,
    next: u64,
}
impl OpaqueIds {
    pub fn get(&mut self, key: &str) -> String {
        if let Some(id) = self.ids.get(key) {
            return id.clone();
        }
        self.next += 1;
        let id = format!("t{}", self.next);
        self.ids.insert(key.into(), id.clone());
        id
    }
    pub fn resolve(&self, id: &str) -> Option<&str> {
        self.ids
            .iter()
            .find(|(_, value)| value.as_str() == id)
            .map(|(key, _)| key.as_str())
    }
}
pub fn context_block(terminals: &[TerminalDescriptor]) -> String {
    let safe: Vec<_> = terminals
        .iter()
        .cloned()
        .map(|mut terminal| {
            terminal.title = crate::redact::redact(&terminal.title);
            terminal.cwd = terminal.cwd.map(|value| crate::redact::redact(&value));
            terminal.status = crate::redact::redact(&terminal.status);
            if let Some(connection) = &mut terminal.connection {
                connection.id = crate::redact::redact(&connection.id);
                connection.name = crate::redact::redact(&connection.name);
                connection.group = connection
                    .group
                    .take()
                    .map(|value| crate::redact::redact(&value));
                connection.description = crate::redact::redact(&connection.description);
                connection.host = crate::redact::redact(&connection.host);
                connection.user = crate::redact::redact(&connection.user);
            }
            terminal
        })
        .collect();
    let json = serde_json::to_string(&safe).expect("descriptors serialize");
    // Preserve JSON while escaping delimiters embedded in user-editable metadata.
    format!(
        "<nocterm_context>\n{}\n</nocterm_context>",
        json.replace('<', "\\u003c").replace('>', "\\u003e")
    )
}
