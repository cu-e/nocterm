//! Best-effort removal of secrets from text an agent will see.
//!
//! Recognizes private key blocks, tokens with a known shape (cloud, VCS,
//! chat and payment providers, JWTs), credentials embedded in URLs, HTTP
//! authorization headers and `name = value` assignments whose name says it
//! holds a secret. Arbitrary secrets cannot be recognized reliably.
mod rules;
use regex::Regex;
pub use rules::{RedactionRule, Replacement, rules as redaction_rules};
use std::sync::LazyLock;

static PRIVATE_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(rules::rules()[0].pattern()).expect("private key pattern"));
static TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(rules::rules()[1].pattern()).expect("token pattern"));
/// `scheme://user:password@host`: keeps the user, hides the password.
static URL_CREDENTIALS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(rules::rules()[2].pattern()).expect("url pattern"));
static AUTHORIZATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(rules::rules()[3].pattern()).expect("authorization pattern"));
static ASSIGNMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(rules::rules()[4].pattern()).expect("secret pattern"));

/// Best effort only: arbitrary secrets cannot be recognized reliably.
pub fn redact(text: &str) -> String {
    let value = PRIVATE_KEY.replace_all(text, "[REDACTED PRIVATE KEY]");
    let value = TOKEN.replace_all(&value, "[REDACTED TOKEN]");
    let value = URL_CREDENTIALS.replace_all(&value, "${1}[REDACTED]@");
    let value = AUTHORIZATION.replace_all(&value, "$1: [REDACTED]");
    ASSIGNMENT
        .replace_all(&value, "$1$2[REDACTED]")
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn hides_tokens_of_known_shapes() {
        for secret in [
            "sk-proj-abcdefghijklmnop",
            "sk-ant-api03-abcdefghijkl",
            "ghp_abcdefghijklmnopqrst",
            "glpat-abcdefghijklmnop",
            "xoxb-1234567890-abcdefgh",
            "sk_live_abcdefghijklmnop",
            "AKIAIOSFODNN7EXAMPLE",
            "AIzaSyA-abcdefghijklmnopqrstuvwxyz012345",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
            "npm_abcdefghijklmnopqrstuvwxyz",
            "hf_abcdefghijklmnopqrstuvwxyz",
        ] {
            let text = format!("value {secret} end");
            let redacted = redact(&text);
            assert!(!redacted.contains(secret), "{secret}: {redacted}");
            assert!(redacted.starts_with("value ") && redacted.ends_with(" end"));
        }
    }

    #[test]
    fn hides_assignments_headers_and_url_passwords_but_keeps_names() {
        let text = "DB_PASSWORD=hunter2 api_key: 'k-123' client_secret=\"abc\" \
                    Authorization: Bearer abc.def postgres://app:s3cret@db:5432/x \
                    \"github_token\": \"value\"";
        let redacted = redact(text);
        for secret in [
            "hunter2",
            "k-123",
            "\"abc\"",
            "abc.def",
            "s3cret",
            "\"value\"",
        ] {
            assert!(!redacted.contains(secret), "{secret}: {redacted}");
        }
        for kept in [
            "DB_PASSWORD=",
            "api_key:",
            "postgres://app:",
            "@db:5432",
            "github_token",
        ] {
            assert!(redacted.contains(kept), "{kept}: {redacted}");
        }
    }

    #[test]
    fn hides_private_key_blocks_including_unterminated_ones() {
        let text = "before\n-----BEGIN RSA PRIVATE KEY-----\nMIIE\n-----END RSA PRIVATE KEY-----\nafter\n-----BEGIN PGP PRIVATE KEY BLOCK-----\nlQ";
        let redacted = redact(text);
        assert!(!redacted.contains("MIIE") && !redacted.contains("lQ"));
        assert!(redacted.contains("before") && redacted.contains("after"));
    }

    #[test]
    fn leaves_ordinary_output_alone() {
        let text = "total 12\ndrwxr-xr-x 2 root root 4096 token_count.rs\nssh user@host -p 22";
        assert_eq!(redact(text), text);
    }
}
