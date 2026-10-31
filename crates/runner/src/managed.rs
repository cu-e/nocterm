//! Managed mode: the pool host starts the runner as
//! `nocterm-runner managed --config <mount>/runner.toml` and writes one boot
//! document to its standard input. The document names what the agent is and
//! points at the host's credential broker. It never carries a provider key, and
//! nothing here writes one to disk.
//!
//! Boot v1 (no `--control`) carries one lease token and is read to EOF; it keeps
//! the runner control-plane only. Boot v2 (`--control stdin`, ADR 0123) carries no
//! token: the framed channel behind it delivers per-chat broker tokens.
mod boot_v2;
mod chat_tokens;
mod frames;
mod providers;
mod session;
#[cfg(test)]
mod testing;
pub(crate) use boot_v2::BootV2;
pub(crate) use chat_tokens::{ChatTokens, ProviderWake};
pub(crate) use frames::{ControlEnd, read_boot_v2, spawn_reader};
pub(crate) use providers::{BrokeredProviders, CoordinatorWake};
pub(crate) use session::{Launch, Session};

use crate::RunnerError;
use nocterm_agent_engine::ProviderEndpoint;
use nocterm_core::SharedCause;
use serde::Deserialize;
use std::io::Read;
use zeroize::{Zeroize, Zeroizing};

/// Largest accepted boot document.
const MAX_BOOT_BYTES: usize = 64 * 1024;
/// The only format version this runner understands.
const BOOT_VERSION: u32 = 1;
const MAX_ENTRIES: usize = 64;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Provider {
    kind: String,
    base_url: String,
    models: Vec<String>,
    token: String,
}
impl Drop for Provider {
    fn drop(&mut self) {
        self.token.zeroize();
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SshTarget {
    host: String,
    user: String,
    port: u16,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    instance: String,
    agent: String,
    lease: String,
    name: String,
    instructions: String,
    provider: Provider,
    secret_names: Vec<String>,
    allowed_hosts: Vec<String>,
    ssh_targets: Vec<SshTarget>,
}

/// What every boot version shares and validates the same way.
struct Shape<'a> {
    identity: [&'a String; 3],
    name: &'a str,
    secret_names: usize,
    allowed_hosts: usize,
    ssh_targets: &'a [SshTarget],
}
fn check_shape(shape: &Shape<'_>) -> Result<(), RunnerError> {
    for text in shape.identity {
        if text.is_empty() || text.len() > 128 || !text.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(RunnerError::Value("boot.identity"));
        }
    }
    if shape.name.is_empty() || shape.name.len() > 256 {
        return Err(RunnerError::Value("boot.name"));
    }
    if shape.secret_names > MAX_ENTRIES
        || shape.allowed_hosts > MAX_ENTRIES
        || shape.ssh_targets.len() > MAX_ENTRIES
        || shape
            .ssh_targets
            .iter()
            .any(|t| t.host.is_empty() || t.user.is_empty() || t.port == 0)
    {
        return Err(RunnerError::Value("boot.lists"));
    }
    Ok(())
}

/// A validated boot document. The virtual token is zeroized when it drops.
pub(crate) struct Boot {
    document: Document,
}
impl Boot {
    /// Reads and validates the document from `input`, refusing anything over
    /// the bound, unknown fields and a provider other than the broker.
    pub(crate) fn read(input: impl Read) -> Result<Self, RunnerError> {
        let mut bytes = Zeroizing::new(Vec::new());
        input
            .take((MAX_BOOT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| RunnerError::Boot(SharedCause::new(error)))?;
        if bytes.len() > MAX_BOOT_BYTES {
            return Err(RunnerError::Value("boot document size"));
        }
        let document: Document = serde_json::from_slice(&bytes)
            .map_err(|error| RunnerError::Boot(SharedCause::new(error)))?;
        Self::prepare(document)
    }
    fn prepare(document: Document) -> Result<Self, RunnerError> {
        if document.version != BOOT_VERSION {
            return Err(RunnerError::Value("boot.version"));
        }
        check_shape(&Shape {
            identity: [&document.instance, &document.agent, &document.lease],
            name: &document.name,
            secret_names: document.secret_names.len(),
            allowed_hosts: document.allowed_hosts.len(),
            ssh_targets: &document.ssh_targets,
        })?;
        let provider = &document.provider;
        if !matches!(provider.kind.as_str(), "openai_chat" | "anthropic_messages")
            || provider.models.is_empty()
            || provider.models.len() > MAX_ENTRIES
            || provider.models.iter().any(String::is_empty)
            || provider.token.is_empty()
            || provider.token.len() > 4096
            || !provider.token.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(RunnerError::Value("boot.provider"));
        }
        // The broker is the only provider endpoint a managed runner may use.
        ProviderEndpoint::broker(&provider.base_url)?;
        Ok(Self { document })
    }
    /// Non-secret summary for the single line the runner reports at start.
    pub(crate) fn summary(&self) -> String {
        let doc = &self.document;
        format!(
            "mode=managed\ninstance={}\nagent={}\nlease={}\ninstructions_bytes={}\nmodels={}\nsecret_names={}\nssh_targets={}\nexecution_ready=false\n",
            doc.instance,
            doc.agent,
            doc.lease,
            doc.instructions.len(),
            doc.provider.models.len(),
            doc.secret_names.len(),
            doc.ssh_targets.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document() -> serde_json::Value {
        serde_json::json!({
            "version": 1, "instance": "inst1", "agent": "agent1", "lease": "lease1",
            "name": "Scribe", "instructions": "be brief",
            "provider": {"kind": "openai_chat", "base_url": "http://10.0.0.2:8080/v1",
                "models": ["gpt-x"], "token": "virtual-token"},
            "secret_names": ["GITHUB"], "allowed_hosts": ["example.org"],
            "ssh_targets": [{"host": "h", "user": "u", "port": 22}]
        })
    }
    fn read(value: &serde_json::Value) -> Result<Boot, RunnerError> {
        Boot::read(value.to_string().as_bytes())
    }
    #[test]
    fn a_host_boot_document_is_accepted_and_summarised_without_the_token() {
        let boot = read(&document()).unwrap();
        let summary = boot.summary();
        assert!(summary.contains("lease=lease1"));
        assert!(!summary.contains("virtual-token"));
    }
    #[test]
    fn unknown_fields_other_versions_and_public_plain_http_are_refused() {
        let mut extra = document();
        extra["api_key"] = "sk-real".into();
        assert!(read(&extra).is_err());
        let mut other = document();
        other["version"] = 2.into();
        assert!(read(&other).is_err());
        let mut public = document();
        public["provider"]["base_url"] = "http://api.example.org/v1".into();
        assert!(read(&public).is_err());
        let mut kind = document();
        kind["provider"]["kind"] = "carrier_pigeon".into();
        assert!(read(&kind).is_err());
        let mut empty = document();
        empty["provider"]["token"] = "".into();
        assert!(read(&empty).is_err());
    }
    #[test]
    fn an_oversized_boot_document_is_refused_before_parsing() {
        assert!(Boot::read(&vec![b' '; MAX_BOOT_BYTES + 1][..]).is_err());
    }
}
