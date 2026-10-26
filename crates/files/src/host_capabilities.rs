//! How many transfers run and how many failed, offered to user plugins that were granted
//! `transfers.counts`.
//!
//! Passive: it reads the snapshot of the transfer service the status bar and the queue already
//! poll, and starts, retries or cancels nothing.
use std::sync::Arc;

use gpui_kit::App;
use nocterm_plugin_api::{
    HostError,
    capability::transfers::{Counts, CountsNow},
};
use nocterm_plugin_model::PluginId;
use nocterm_plugin_ports::Observe;
use nocterm_transfers::TransferService;

use crate::Installed;

/// The counts of the transfer queue shared by every window.
pub struct TransferCounts {
    service: Arc<dyn TransferService>,
}

impl TransferCounts {
    pub fn new(installed: &Installed) -> Self {
        Self::over(installed.transfers().clone())
    }

    /// The counts of `service`, for a composition that holds the service without [`Installed`].
    pub fn over(service: Arc<dyn TransferService>) -> Self {
        Self { service }
    }
}

impl Observe<CountsNow> for TransferCounts {
    /// Transfers not yet finished, and transfers in which a file failed.
    fn sample(&self, _: &PluginId, _: &App) -> Result<Counts, HostError> {
        let snapshot = self.service.snapshot();
        let count = |matching: &dyn Fn(&nocterm_transfers::Progress) -> bool| {
            u16::try_from(snapshot.iter().filter(|job| matching(job)).count()).unwrap_or(u16::MAX)
        };
        Ok(Counts {
            active: count(&|job| !job.state.finished()),
            failed: count(&|job| job.failed_files > 0),
        })
    }
}

#[cfg(test)]
mod tests;
