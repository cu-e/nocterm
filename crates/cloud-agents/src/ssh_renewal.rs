//! Keeps a tracked hub's bound Cloud chats answered (ADR 0139): the runner of a chat bound to
//! the person's servers asks for a short certificate at 80 % of the current one's life, and an
//! unlocked device of the account answers it inside the binding. While a hub is tracked, its
//! certifier is swept at once and then every [`SWEEP_INTERVAL`]; a sweep that fails (the account
//! is locked or the hub unreachable) is tried again at the next one. A hub that does not offer
//! Cloud chats stops being swept; an untracked hub is forgotten.
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Context, Task};
use nocterm_cloud_agents_model::{ChatSshCertifier, CloudAgentError};
use nocterm_hubs_model::HubId;

use crate::service::CloudAgents;

/// How often the bound chats of a hub are swept: well inside the shortest renewal margin of a
/// certificate (20 % of a one-hour certificate).
pub(crate) const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// The sweeps of the tracked hubs.
#[derive(Default)]
pub(crate) struct SshRenewals {
    hubs: BTreeMap<HubId, Task<()>>,
}
impl SshRenewals {
    /// Starts sweeping `hub`; a hub already swept keeps its sweep.
    pub(crate) fn start(
        &mut self,
        hub: &HubId,
        certifier: Arc<dyn ChatSshCertifier>,
        cx: &mut Context<CloudAgents>,
    ) {
        if self.hubs.contains_key(hub) {
            return;
        }
        let hub_id = hub.clone();
        let sweeping = cx.spawn(async move |_, cx| {
            loop {
                match certifier.sweep(&hub_id).await {
                    Ok(read) => tracing::debug!(read, "bound Cloud chats are answered"),
                    Err(CloudAgentError::Unsupported) => return,
                    Err(error) => tracing::debug!(%error, "bound Cloud chats were not swept"),
                }
                cx.background_executor().timer(SWEEP_INTERVAL).await;
            }
        });
        self.hubs.insert(hub.clone(), sweeping);
    }
    /// Stops sweeping `hub` and lets its chats go; whether it was swept.
    pub(crate) fn stop(&mut self, hub: &HubId, certifier: &dyn ChatSshCertifier) -> bool {
        let swept = self.hubs.remove(hub).is_some();
        if swept {
            certifier.forget(hub);
        }
        swept
    }
}
