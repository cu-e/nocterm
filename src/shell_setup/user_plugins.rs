//! The application's side of the user plugin host: where plugin documents live on disk and the
//! providers of one window.
use nocterm_core::fs::BoundedReadError;

mod data;
mod providers;
mod records;

pub(crate) use data::PluginDataFiles;
pub(crate) use providers::window_providers;
pub(crate) use records::RecordsFile;

/// Whether a failed bounded read means the file is not there at all.
fn is_missing(error: &BoundedReadError) -> bool {
    error.io_kind() == Some(std::io::ErrorKind::NotFound)
}

#[cfg(test)]
mod tests;
