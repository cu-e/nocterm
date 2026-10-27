//! Opening the windows the previous run left open.

use gpui_kit::App;
use nocterm_hubs_model::{Environment, WindowRecord};
use nocterm_hubs_ui::EnvironmentOpenError;

use crate::bootstrap::Booted;

/// Opens a window for each of `previous`, the windows the last run left open, in order. A hub
/// window comes back with the organization and sessions it kept.
///
/// A window that cannot open is skipped. A server that is no longer in `[hubs]` is forgotten;
/// one that is unavailable only in this run, because the settings could not be read or hubs
/// did not start, stays in the saved list. When none opens, or nothing was remembered, one
/// local window opens, so the application always starts with a window. Only that fallback
/// failing is an error.
pub(crate) fn restore_windows(
    cx: &mut App,
    booted: &Booted,
    previous: &[WindowRecord],
) -> Result<(), EnvironmentOpenError> {
    let mut opened = 0;
    for record in previous {
        match super::open_remembered(cx, booted, record.clone()) {
            Ok(()) => opened += 1,
            Err(error) => {
                tracing::warn!(%error, environment = ?record.environment(), "a window was not restored");
                if matches!(error, EnvironmentOpenError::UnknownHub(_)) {
                    booted.windowing().not_restored(record.clone(), cx);
                }
            }
        }
    }
    if opened == 0 {
        super::open_window(cx, booted, Environment::Local)?;
    }
    Ok(())
}
