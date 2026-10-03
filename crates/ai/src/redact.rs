use regex::Regex;
use std::sync::LazyLock;
static PEM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?(?:-----END [A-Z ]*PRIVATE KEY-----|$)")
        .expect("PEM pattern")
});
static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:sk-(?:ant-)?|gh[pousr]_|github_pat_|xox[baprs]-)[A-Za-z0-9_-]{8,}")
        .expect("token pattern")
});
static ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b(password|passwd|api[_-]?key|access[_-]?token|authorization)\s*[:=]\s*(?:"[^"\r\n]*"|'[^'\r\n]*'|(?:Bearer\s+)?[^\s,;"']+)"#).expect("secret pattern")
});
/// Best effort only: arbitrary secrets cannot be recognized reliably.
pub fn redact(text: &str) -> String {
    let value = PEM.replace_all(text, "[REDACTED PRIVATE KEY]");
    let value = TOKEN.replace_all(&value, "[REDACTED TOKEN]");
    ASSIGNMENT.replace_all(&value, "$1=[REDACTED]").into_owned()
}
