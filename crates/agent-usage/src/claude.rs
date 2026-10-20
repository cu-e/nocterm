//! Claude's account usage, read with Claude Code's own OAuth token.
use futures::future::BoxFuture;
use nocterm_ai::usage::{AccountUsageSource, RateLimit, UsageAgent, UsageReadError};
use std::{
    io::Read as _,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BODY_BYTES: u64 = 1024 * 1024;

/// Reads Claude's plan limits under a configuration directory supplied by
/// composition.
pub struct ClaudeUsageReader {
    config: Option<PathBuf>,
}

impl ClaudeUsageReader {
    pub fn new(config: Option<PathBuf>) -> Self {
        Self { config }
    }
}

impl AccountUsageSource for ClaudeUsageReader {
    fn read(
        &self,
        agent: UsageAgent,
    ) -> BoxFuture<'static, Result<Option<Vec<RateLimit>>, UsageReadError>> {
        let config = self.config.clone();
        Box::pin(async move {
            let claude = agent.id == "claude"
                || std::iter::once(&agent.command)
                    .chain(&agent.args)
                    .any(|part| part.contains("claude-agent-acp"));
            match config.filter(|_| claude) {
                Some(config) => match token(&config.join(".credentials.json"))? {
                    Some(token) => fetch(&token),
                    None => Ok(None),
                },
                None => Ok(None),
            }
        })
    }
}

/// The unexpired access token in Claude Code's credentials, if it keeps one
/// there.
fn token(path: &Path) -> Result<Option<String>, UsageReadError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(UsageReadError::read(path.to_owned(), error)),
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Ok(None);
    };
    let oauth = &value["claudeAiOauth"];
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |now| now.as_millis());
    let expired = oauth["expiresAt"]
        .as_u64()
        .is_some_and(|expires| u128::from(expires) <= now);
    Ok(oauth["accessToken"]
        .as_str()
        .filter(|token| !token.is_empty() && !expired)
        .map(str::to_owned))
}

fn fetch(token: &str) -> Result<Option<Vec<RateLimit>>, UsageReadError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("nocterm/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| UsageReadError::fetch(USAGE_URL, error))?;
    let response = client
        .get(USAGE_URL)
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("Content-Type", "application/json")
        .send()
        .map_err(|error| UsageReadError::fetch(USAGE_URL, error))?;
    // A rejected or unsupported account reports nothing rather than failing.
    if !response.status().is_success() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_BODY_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|error| UsageReadError::fetch(USAGE_URL, error))?;
    let body: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| UsageReadError::fetch(USAGE_URL, error))?;
    Ok(crate::parser::claude_limits(&body))
}

#[cfg(test)]
mod tests {
    use super::{ClaudeUsageReader, token};
    use nocterm_ai::usage::{AccountUsageSource as _, UsageAgent};

    fn agent(id: &str) -> UsageAgent {
        UsageAgent {
            id: id.into(),
            command: "npx".into(),
            args: Vec::new(),
        }
    }

    #[test]
    fn other_agents_and_missing_credentials_report_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let reader = ClaudeUsageReader::new(Some(dir.path().to_owned()));
        let read = |id| futures::executor::block_on(reader.read(agent(id)));
        assert_eq!(read("codex").unwrap(), None);
        assert_eq!(read("claude").unwrap(), None);
        let unconfigured = ClaudeUsageReader::new(None);
        assert_eq!(
            futures::executor::block_on(unconfigured.read(agent("claude"))).unwrap(),
            None
        );
    }

    #[test]
    fn only_an_unexpired_token_is_used() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".credentials.json");
        let write = |json: serde_json::Value| std::fs::write(&path, json.to_string()).unwrap();
        write(
            serde_json::json!({"claudeAiOauth": {"accessToken": "fresh", "expiresAt": u64::MAX}}),
        );
        assert_eq!(token(&path).unwrap().as_deref(), Some("fresh"));
        write(serde_json::json!({"claudeAiOauth": {"accessToken": "stale", "expiresAt": 1}}));
        assert_eq!(token(&path).unwrap(), None);
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(token(&path).unwrap(), None);
    }
}
