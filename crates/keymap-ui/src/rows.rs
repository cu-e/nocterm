//! The table's rows: every binding in effect, then every command without
//! one, and the search over them.
use gpui_kit::{App, SharedString};
use nocterm_keymap::{Binding, Keymap, Source, humanize, normalize, offered};

/// One line of the table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub action: String,
    pub label: SharedString,
    pub description: SharedString,
    /// `None` for a command with no binding.
    pub keystrokes: Option<String>,
    pub context: Option<String>,
    pub source: Option<Source>,
    /// Whether the user changed this command's bindings.
    pub customized: bool,
}

/// The context a new binding of `action` goes in: where its defaults are,
/// else where its feature's view is.
pub(crate) fn default_context(action: &str, defaults: &[Binding]) -> Option<String> {
    if let Some(binding) = defaults.iter().find(|binding| binding.action == action) {
        return binding.context.clone();
    }
    Some(
        match action.split_once("::").map(|(namespace, _)| namespace) {
            Some("terminal") => "Terminal",
            Some("agent") => "AgentPanel",
            _ => "Workspace",
        }
        .to_owned(),
    )
}

pub(crate) fn rows(cx: &App) -> Vec<Row> {
    let keymap = Keymap::global(cx);
    let docs = cx.action_documentation();
    let row = |action: &str, binding: Option<&Binding>| Row {
        action: action.to_owned(),
        label: humanize(action).into(),
        description: docs
            .get(action)
            .map(|doc| doc.trim().trim_end_matches('.').to_owned())
            .unwrap_or_default()
            .into(),
        keystrokes: binding.map(|binding| binding.keystrokes.clone()),
        context: binding.and_then(|binding| binding.context.clone()),
        source: binding.map(|binding| binding.source),
        customized: keymap.is_customized(action),
    };
    let bindings = keymap.bindings();
    let mut rows: Vec<Row> = bindings
        .iter()
        .filter(|binding| offered(&binding.action))
        .map(|binding| row(&binding.action, Some(binding)))
        .collect();
    let mut unbound: Vec<&str> = cx
        .all_action_names()
        .iter()
        .copied()
        .filter(|name| offered(name))
        .filter(|name| !bindings.iter().any(|binding| binding.action == *name))
        .collect();
    unbound.sort_unstable();
    unbound.dedup();
    rows.extend(unbound.into_iter().map(|action| row(action, None)));
    rows.sort_by(|a, b| a.label.cmp(&b.label).then(a.keystrokes.cmp(&b.keystrokes)));
    rows
}

/// Whether `row` matches `query`: every word of it appears in the command's
/// name, description, shortcut or context, ignoring case. A query that reads
/// as a shortcut (`ctrl-shift-p`, `shift-ctrl-p`) also matches the shortcut
/// however its modifiers are ordered.
pub(crate) fn matches(row: &Row, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    if query.contains('-')
        && let (Ok(wanted), Some(keys)) = (normalize(&query), &row.keystrokes)
        && normalize(keys).is_ok_and(|keys| keys.to_lowercase() == wanted.to_lowercase())
    {
        return true;
    }
    let haystack = format!(
        "{} {} {} {} {}",
        row.label,
        row.action,
        row.description,
        row.keystrokes.as_deref().unwrap_or("unbound"),
        row.context.as_deref().unwrap_or_default(),
    )
    .to_lowercase();
    query.split_whitespace().all(|word| haystack.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(keys: Option<&str>) -> Row {
        Row {
            action: "workspace::SwapSides".into(),
            label: "Workspace: Swap Sides".into(),
            description: "Put the sidebar and the AI panel on each other's side".into(),
            keystrokes: keys.map(str::to_owned),
            context: Some("Workspace".into()),
            source: Some(Source::Default),
            customized: false,
        }
    }

    #[test]
    fn search_matches_words_anywhere_and_shortcuts_in_any_order() {
        let bound = row(Some("ctrl-shift-e"));
        assert!(matches(&bound, "swap"));
        assert!(matches(&bound, "SIDEBAR panel"));
        assert!(matches(&bound, "shift-ctrl-e"));
        assert!(matches(&bound, "ctrl-shift"));
        assert!(!matches(&bound, "swap terminal"));
        assert!(matches(&row(None), "unbound"));
    }

    #[test]
    fn new_bindings_go_where_the_feature_lives() {
        assert_eq!(
            default_context("terminal::Copy", &[]).as_deref(),
            Some("Terminal")
        );
        assert_eq!(
            default_context("vault::Thing", &[]).as_deref(),
            Some("Workspace")
        );
    }
}
