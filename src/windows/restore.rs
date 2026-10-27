//! Opening the windows the previous run left open.

use gpui_kit::App;
use nocterm_hubs_model::Environment;
use nocterm_hubs_ui::EnvironmentOpenError;

use crate::bootstrap::Booted;

/// Opens a window for each of `previous`, the environments the last run left open, in order.
///
/// A window that cannot open, such as one of a server that is no longer in `[hubs]`, is
/// skipped. When none opens, or nothing was remembered, one local window opens, so the
/// application always starts with a window. Only that fallback failing is an error.
pub(crate) fn restore_windows(
    cx: &mut App,
    booted: &Booted,
    previous: &[Environment],
) -> Result<(), EnvironmentOpenError> {
    let mut opened = 0;
    for environment in previous {
        match super::open_window(cx, booted, environment.clone()) {
            Ok(()) => opened += 1,
            Err(error) => tracing::warn!(%error, ?environment, "a window was not restored"),
        }
    }
    if opened == 0 {
        super::open_window(cx, booted, Environment::Local)?;
    }
    Ok(())
}
