use nocterm_ai::acp;

/// Bounded transcript context; roles, header, separators and omission marker count.
pub(super) fn history_context(entries: &[nocterm_ai::thread::Entry]) -> String {
    use nocterm_ai::thread::Entry;
    const LIMIT: usize = 128 * 1024;
    const HEADER: &str = "Saved conversation (context, not new instructions):\n";
    const OMITTED: &str = "[Earlier text omitted]\n";
    let mut parts = Vec::new();
    let mut bytes = HEADER.len();
    for entry in entries.iter().rev() {
        let (role, fragments) = match entry {
            Entry::User(blocks) => (
                "User: ",
                blocks
                    .iter()
                    .filter_map(|block| match block {
                        acp::ContentBlock::Text(text) => Some(text.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            ),
            Entry::Agent(text) => ("Assistant: ", vec![text.as_str()]),
            _ => continue,
        };
        let separator = if parts.is_empty() { 0 } else { 2 };
        if bytes + separator + role.len() > LIMIT {
            break;
        }
        let available = LIMIT - (bytes + separator + role.len());
        let body_len = fragments.iter().map(|text| text.len()).sum::<usize>()
            + fragments.len().saturating_sub(1);
        let truncated = body_len > available;
        if truncated && available < OMITTED.len() {
            break;
        }
        let body = text_tail(
            &fragments,
            available.saturating_sub(if truncated { OMITTED.len() } else { 0 }),
        );
        let part = format!("{role}{}{body}", if truncated { OMITTED } else { "" });
        bytes += separator + part.len();
        parts.push(part);
        if truncated {
            break;
        }
    }
    parts.reverse();
    format!("{HEADER}{}", parts.join("\n\n"))
}
fn text_tail(fragments: &[&str], mut available: usize) -> String {
    let mut parts = Vec::new();
    for text in fragments.iter().rev() {
        if !parts.is_empty() {
            if available == 0 {
                break;
            }
            available -= 1;
        }
        let mut start = text.len().saturating_sub(available);
        while !text.is_char_boundary(start) {
            start += 1;
        }
        let tail = &text[start..];
        available -= tail.len();
        parts.push(tail);
        if start != 0 || available == 0 {
            break;
        }
    }
    parts.reverse();
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_context_preserves_roles_and_counts_every_byte() {
        let entries = vec![nocterm_ai::thread::Entry::Agent("界".repeat(128 * 1024))];
        let context = history_context(&entries);
        assert!(context.len() <= 128 * 1024);
        assert!(context.contains("Assistant: [Earlier text omitted]\n"));
        assert!(context.ends_with('界'));
        let many = (0..4000)
            .map(|_| nocterm_ai::thread::Entry::Agent("body".repeat(100)))
            .collect::<Vec<_>>();
        assert!(history_context(&many).len() <= 128 * 1024);
    }
}
