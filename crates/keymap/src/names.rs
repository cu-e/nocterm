//! How actions are named to people, and which of them are user commands.

/// Action namespaces of nocterm's own commands. The component library's
/// actions (text input, lists, menus) only make sense inside their widget.
pub const OFFERED_NAMESPACES: &[&str] = &[
    "workspace",
    "terminal",
    "connections",
    "files",
    "agent",
    "vault",
];

/// Whether `name` is a command a person runs: in nocterm's namespaces, and
/// not the command palette itself.
pub fn offered(name: &str) -> bool {
    name != "workspace::ToggleCommandPalette"
        && name
            .split_once("::")
            .is_some_and(|(namespace, _)| OFFERED_NAMESPACES.contains(&namespace))
}

/// `workspace::ToggleRightPanel` → `Workspace: Toggle Right Panel`.
pub fn humanize(name: &str) -> String {
    let (namespace, action) = name.rsplit_once("::").unwrap_or(("", name));
    let chars: Vec<char> = action.chars().collect();
    let mut words = String::new();
    for (index, ch) in chars.iter().enumerate() {
        let previous_lower = index > 0 && chars[index - 1].is_lowercase();
        let next_lower = chars.get(index + 1).is_some_and(|c| c.is_lowercase());
        if index > 0 && ch.is_uppercase() && (previous_lower || next_lower) {
            words.push(' ');
        }
        words.push(*ch);
    }
    let namespace = namespace.rsplit("::").next().unwrap_or_default();
    let mut first = namespace.chars();
    match first.next() {
        Some(letter) => format!("{}{}: {words}", letter.to_uppercase(), first.as_str()),
        None => words,
    }
}

#[cfg(test)]
mod tests {
    use super::{humanize, offered};

    #[test]
    fn names_read_as_words() {
        assert_eq!(
            humanize("workspace::ToggleRightPanel"),
            "Workspace: Toggle Right Panel"
        );
        assert_eq!(
            humanize("workspace::OpenSSHSettings"),
            "Workspace: Open SSH Settings"
        );
        assert_eq!(humanize("agent::NewThread"), "Agent: New Thread");
    }

    #[test]
    fn only_nocterm_commands_are_offered() {
        assert!(offered("terminal::Copy"));
        assert!(!offered("input::Backspace"));
        assert!(!offered("workspace::ToggleCommandPalette"));
    }
}
