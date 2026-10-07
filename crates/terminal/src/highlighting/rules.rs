//! Ordered field rules: specific messages win over structural tokens.
use super::Role;
use regex::Regex;
use std::{net::IpAddr, sync::LazyLock};

pub(super) struct Token {
    pub start: usize,
    pub end: usize,
    pub role: Role,
}
struct Rule {
    regex: Regex,
    role: Role,
    address: bool,
}
const MAX_TOKENS: usize = 512;
static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    [
        (r"(?i)\b(?:failed password|authentication failure|invalid user|couldn't stat|no such file or directory|no warranty|permission denied|fatal|error|failed|failure)\b", Role::Error, false),
        (r"(?i)\b(?:warning|warn|deprecated|timeout|timed out)\b", Role::Warning, false),
        (r"(?i)\b(?:accepted password|accepted publickey|success(?:ful(?:ly)?)?)\b", Role::Success, false),
        (r"(?i)\b(?:connection closed|connected|session opened|session closed)\b", Role::Info, false),
        (r"(?:[0-9]{1,3}\.){3}[0-9]{1,3}|[0-9a-fA-F]*:[0-9a-fA-F:.]*:[0-9a-fA-F:.]*", Role::Address, true),
        (r"\b(?:19|20)[0-9]{2}-(?:0[1-9]|1[0-2])-(?:0[1-9]|[12][0-9]|3[01])(?:[T ](?:[01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9](?:\.[0-9]+)?(?:Z|[+-][0-2][0-9]:[0-5][0-9])?)?\b|\b(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) +(?:[1-9]|0[1-9]|[12][0-9]|3[01])\b|\b(?:[01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9](?:\.[0-9]+)?\b", Role::Timestamp, false),
        (r"\b[0-9]+\.[0-9]+\.[0-9]+(?:[+~-][[:alnum:]_.+~-]+)?\b", Role::Version, false),
        (r"\b[[:alnum:]_.-]+\[(?P<value>[0-9]+)\]|\b(?:uid|gid|pid|port)[ =](?P<number>[0-9]+)\b", Role::Identifier, false),
    ].into_iter().map(|(pattern, role, address)| Rule {
        regex: Regex::new(pattern).expect("valid terminal highlight rule"), role, address,
    }).collect()
});

// Use the regex engine's Unicode word definition for every field family,
// including marks and join controls; ASCII log boundaries need no extra regex.
static UNICODE_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A\w\z").expect("valid Unicode word character rule"));

fn word_character(ch: char) -> bool {
    if ch.is_ascii() {
        return ch.is_ascii_alphanumeric() || ch == '_';
    }
    let mut bytes = [0; 4];
    UNICODE_WORD.is_match(ch.encode_utf8(&mut bytes))
}

fn boundary(text: &str, start: usize, end: usize, address: bool) -> bool {
    let part = |ch: char| word_character(ch) || (address && (ch == '.' || ch == ':'));
    !text[..start].chars().next_back().is_some_and(part)
        && !text[end..].chars().next().is_some_and(part)
}

pub(super) fn recognize(text: &str, start_complete: bool, end_complete: bool) -> Vec<Token> {
    let mut tokens = Vec::new();
    for rule in RULES.iter() {
        for captures in rule.regex.captures_iter(text) {
            let whole = captures.get(0).expect("full match");
            if (!start_complete && whole.start() == 0)
                || (!end_complete && whole.end() == text.len())
            {
                continue;
            }
            if !boundary(
                text,
                whole.start(),
                whole.end(),
                rule.address || rule.role == Role::Version,
            ) {
                continue;
            }
            if rule.address && whole.as_str().parse::<IpAddr>().is_err() {
                continue;
            }
            let value = captures
                .name("value")
                .or_else(|| captures.name("number"))
                .unwrap_or(whole);
            tokens.push(Token {
                start: value.start(),
                end: value.end(),
                role: rule.role,
            });
            if tokens.len() == MAX_TOKENS {
                return tokens;
            }
        }
    }
    tokens
}
