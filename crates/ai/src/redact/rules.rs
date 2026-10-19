//! Canonical immutable rule data; native engines own compilation and scratch.
#[derive(Clone, Copy)]
pub enum Replacement {
    Literal(&'static str),
    CaptureOne(&'static str),
    CaptureOneTwo(&'static str),
}
#[derive(Clone, Copy)]
pub struct RedactionRule {
    pattern: &'static str,
    replacement: Replacement,
}
impl RedactionRule {
    pub fn pattern(&self) -> &'static str {
        self.pattern
    }
    pub fn replacement(&self) -> Replacement {
        self.replacement
    }
}
pub fn rules() -> [RedactionRule; 5] {
    [
        RedactionRule {
            pattern: r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----.*?(?:-----END [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----|$)",
            replacement: Replacement::Literal("[REDACTED PRIVATE KEY]"),
        },
        RedactionRule {
            pattern: concat!(
                r"\b(?:",
                // OpenAI, Anthropic and compatible API keys.
                r"sk-(?:ant-|proj-)?[A-Za-z0-9_-]{8,}",
                // GitHub, GitLab, npm, Hugging Face, PyPI.
                r"|gh[pousr]_[A-Za-z0-9]{8,}|github_pat_[A-Za-z0-9_]{8,}|glpat-[A-Za-z0-9_-]{8,}",
                r"|npm_[A-Za-z0-9]{16,}|hf_[A-Za-z0-9]{16,}|pypi-[A-Za-z0-9_-]{16,}",
                // Slack, Stripe, SendGrid, Twilio.
                r"|xox[abprs]-[A-Za-z0-9-]{8,}|(?:sk|rk)_(?:live|test)_[A-Za-z0-9]{8,}",
                r"|SG\.[A-Za-z0-9_-]{16,}\.[A-Za-z0-9_-]{16,}|SK[0-9a-f]{32}",
                // AWS access key ids, Google API keys.
                r"|(?:AKIA|ASIA|AGPA|AIDA|AROA)[A-Z0-9]{16}|AIza[0-9A-Za-z_-]{35}",
                // JSON Web Tokens.
                r"|eyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
                r")",
            ),
            replacement: Replacement::Literal("[REDACTED TOKEN]"),
        },
        RedactionRule {
            pattern: r"(?i)\b([a-z][a-z0-9+.-]*://[^\s:/@]+:)[^\s@/]+@",
            replacement: Replacement::CaptureOne("[REDACTED]@"),
        },
        RedactionRule {
            pattern: r"(?i)\b(authorization|proxy-authorization)\s*:\s*(?:bearer|basic|token)?\s*[^\s,;]+",
            replacement: Replacement::CaptureOne(": [REDACTED]"),
        },
        RedactionRule {
            pattern: concat!(
                r#"(?i)\b([a-z0-9_.-]*(?:password|passwd|passphrase|secret|api[_-]?key|access[_-]?key|"#,
                r#"private[_-]?key|access[_-]?token|auth[_-]?token|refresh[_-]?token|client[_-]?secret|token)[a-z0-9_.-]*)"#,
                r#"(["']?\s*[:=]\s*)(?:"[^"\r\n]*"|'[^'\r\n]*'|(?:bearer\s+)?[^\s,;"']+)"#,
            ),
            replacement: Replacement::CaptureOneTwo("[REDACTED]"),
        },
    ]
}
