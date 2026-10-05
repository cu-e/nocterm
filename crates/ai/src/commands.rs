//! Advertised slash commands share one interpretation in UI and prompt dispatch.
use crate::acp;
use std::ops::Range;
pub struct Invocation<'a> {
    pub name: &'a str,
    pub range: Range<usize>,
    pub has_arguments: bool,
}
pub fn invocation<'a>(text: &'a str, commands: &[acp::AvailableCommand]) -> Option<Invocation<'a>> {
    let trimmed = text.trim_start();
    let start = text.len() - trimmed.len();
    let body = trimmed.strip_prefix('/')?;
    let end = body.find(char::is_whitespace).unwrap_or(body.len());
    let name = &body[..end];
    commands
        .iter()
        .any(|command| command.name == name)
        .then_some(Invocation {
            name,
            range: start..start + 1 + end,
            has_arguments: end < body.len(),
        })
}
/// Suggestions stop once the user starts arguments. Unknown slash text is ordinary input.
pub fn prefix(text: &str) -> Option<&str> {
    let prefix = text.trim_start().strip_prefix('/')?;
    (!prefix.chars().any(char::is_whitespace)).then_some(prefix)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_advertised_names_have_administrative_semantics() {
        let commands = vec![
            acp::AvailableCommand::new("review", "Review"),
            acp::AvailableCommand::new("$skills:界", "Skill"),
        ];
        assert!(invocation("/var/log/nginx/error.log проверь", &commands).is_none());
        assert!(invocation("/unknown", &commands).is_none());
        assert!(invocation("/reviewer", &commands).is_none());
        let parsed = invocation(" \t/$skills:界 args", &commands).unwrap();
        assert_eq!(parsed.name, "$skills:界");
        assert_eq!(&" \t/$skills:界 args"[parsed.range], "/$skills:界");
        assert!(parsed.has_arguments);
        assert_eq!(prefix("  /re"), Some("re"));
        assert_eq!(prefix("/review args"), None);
    }
}
