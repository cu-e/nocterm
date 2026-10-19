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
/// A saved server attached to a chat that has no open session. An agent
/// opens one in the background with the `open_terminal` tool.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ServerDescriptor {
    pub server_id: String,
    pub name: String,
    pub group: Option<String>,
    pub description: String,
    pub host: String,
    pub port: u16,
    pub user: String,
}

/// Short ids handed to agents instead of internal ones.
pub struct OpaqueIds {
    ids: BTreeMap<String, String>,
    next: u64,
    prefix: &'static str,
}
impl Default for OpaqueIds {
    fn default() -> Self {
        Self::with_prefix("t")
    }
}
impl OpaqueIds {
    /// Ids such as `s1`, `s2` for `prefix` `s`.
    pub fn with_prefix(prefix: &'static str) -> Self {
        Self {
            ids: BTreeMap::new(),
            next: 0,
            prefix,
        }
    }
    pub fn get(&mut self, key: &str) -> String {
        if let Some(id) = self.ids.get(key) {
            return id.clone();
        }
        self.next += 1;
        let id = format!("{}{}", self.prefix, self.next);
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
/// How an agent may reach the user's servers, sent with each prompt ahead of
/// the [`context_block`].
pub const TERMINAL_RULES: &str = "Reach the user's terminals and servers only through the nocterm tools, and only those listed in <nocterm_context>. nocterm holds their passwords and keys: never connect on your own with ssh, scp or similar. A closed server terminal reconnects by itself when you use it, and an offline server connects with open_terminal: use them instead of asking the user. Only when a terminal or server is missing or still cannot be connected, tell the user right away in one short sentence (which one and why), instead of trying workarounds.";

/// The terminals and offline servers of a chat, as JSON between delimiters
/// the agent can recognize. User-editable text is filtered for secrets.
pub fn context_block(terminals: &[TerminalDescriptor], servers: &[ServerDescriptor]) -> String {
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
    let servers: Vec<_> = servers
        .iter()
        .cloned()
        .map(|mut server| {
            server.name = crate::redact::redact(&server.name);
            server.group = server.group.map(|value| crate::redact::redact(&value));
            server.description = crate::redact::redact(&server.description);
            server.host = crate::redact::redact(&server.host);
            server.user = crate::redact::redact(&server.user);
            server
        })
        .collect();
    let json = if servers.is_empty() {
        serde_json::to_string(&safe)
    } else {
        serde_json::to_string(&serde_json::json!({
            "terminals": safe,
            "offline_servers": servers,
            "note": "Offline servers have no session yet. Call open_terminal with a server_id to connect in the background, then use the returned terminal_id.",
        }))
    }
    .expect("descriptors serialize");
    // Preserve JSON while escaping delimiters embedded in user-editable metadata.
    format!(
        "<nocterm_context>\n{}\n</nocterm_context>",
        json.replace('<', "\\u003c").replace('>', "\\u003e")
    )
}
