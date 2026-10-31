//! One agent as the hub lists it: descriptive data only.

use crate::ids::{AgentId, PoolId};
use crate::placement::Placement;
use crate::scope::AgentScope;
use crate::status::AgentStatus;

/// What one seat of the agent asks of a host and when it is stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentLimits {
    /// CPU in thousandths of a core while the agent works.
    pub cpu_milli: u32,
    /// Memory in MiB while the agent works.
    pub mem_mib: u32,
    /// Disk reserved for the seat, in MiB.
    pub disk_mib: u32,
    /// Runs the seat may execute at once.
    pub max_parallel_runs: u32,
    /// Seconds without work before the host stops the runner.
    pub idle_ttl_s: u32,
}

/// A pool that holds the agent's sealed definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolLabel {
    /// The pool's identifier.
    pub id: PoolId,
    /// The name the hub gives the pool.
    pub name: String,
}

/// An agent of a hub that the signed-in account may see.
///
/// The listing is descriptive: the sealed definition decides what a host executes, so a hub
/// that alters a listing cannot change behaviour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloudAgent {
    /// The hub's identifier.
    pub id: AgentId,
    /// Whose agent it is.
    pub scope: AgentScope,
    /// Monotonic version; a write names the version it replaces.
    pub version: u64,
    /// Display name.
    pub name: String,
    /// Short description, possibly empty.
    pub description: String,
    /// Icon name, possibly empty.
    pub icon: String,
    /// Provider family for display, for example `openai_chat`.
    pub provider: String,
    /// Model ids for display; the first is the default.
    pub models: Vec<String>,
    /// Resources and idle time.
    pub limits: AgentLimits,
    /// What the person can do now.
    pub status: AgentStatus,
    /// The pools that can run it.
    pub pools: Vec<PoolLabel>,
}

impl CloudAgent {
    /// Where the agent runs. A personal agent no pool holds works only while a device of the
    /// person is online, so it runs on the person's own runner; every other agent runs on a
    /// pool.
    pub fn placement(&self) -> Placement {
        if self.scope == AgentScope::Personal && self.pools.is_empty() {
            Placement::OwnRunner
        } else {
            Placement::Pool
        }
    }

    /// The model shown in the row.
    pub fn primary_model(&self) -> Option<&str> {
        self.models.first().map(String::as_str)
    }

    /// Whether the agent matches the text a person typed in the search field: every word must
    /// occur in the name, a model, the description or the provider, ignoring case.
    pub fn matches(&self, query: &str) -> bool {
        let haystack = self.search_text();
        query
            .split_whitespace()
            .all(|word| haystack.contains(&word.to_lowercase()))
    }

    fn search_text(&self) -> String {
        let mut text = self.name.to_lowercase();
        for part in [&self.description, &self.provider]
            .into_iter()
            .chain(self.models.iter())
        {
            text.push(' ');
            text.push_str(&part.to_lowercase());
        }
        text
    }
}
