//! How the status of a server reads, and in what colour, wherever a server is listed.

use gpui_kit::{App, Hsla, SharedString, component::ActiveTheme as _};
use nocterm_hubs::HubStatus;
use nocterm_ui::IconName;

/// The status as a short phrase with the colour it is drawn in.
pub(crate) fn status_chip(status: &HubStatus, cx: &App) -> (SharedString, Hsla) {
    let theme = cx.theme();
    let now = cx.background_executor().now();
    chip(
        status,
        now,
        theme.success,
        theme.muted_foreground,
        theme.warning,
        theme.danger,
    )
}

/// How a server's status reads in the list, and in what colour.
pub(crate) fn chip(
    status: &HubStatus,
    now: std::time::Instant,
    good: Hsla,
    quiet: Hsla,
    warn: Hsla,
    bad: Hsla,
) -> (SharedString, Hsla) {
    match status {
        HubStatus::Online => ("Online".into(), good),
        HubStatus::Connecting => ("Connecting…".into(), quiet),
        HubStatus::Offline { retry_at } => {
            let seconds = retry_at.saturating_duration_since(now).as_secs_f64().ceil();
            (format!("Offline · retry in {seconds:.0} s").into(), warn)
        }
        HubStatus::SignedOut => ("Sign-in required".into(), warn),
        HubStatus::Revoked => ("Device revoked".into(), bad),
        HubStatus::TrustFailed { .. } => ("Certificate changed".into(), bad),
        HubStatus::UpgradeRequired { hub, client } if hub < client => {
            ("Update the server".into(), warn)
        }
        HubStatus::UpgradeRequired { .. } => ("Update Nocterm".into(), warn),
    }
}

pub(crate) fn status_icon(status: &HubStatus) -> IconName {
    match status {
        HubStatus::Online | HubStatus::Connecting | HubStatus::SignedOut => IconName::Server,
        HubStatus::Offline { .. } => IconName::ServerOff,
        HubStatus::Revoked | HubStatus::TrustFailed { .. } | HubStatus::UpgradeRequired { .. } => {
            IconName::TriangleAlert
        }
    }
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
