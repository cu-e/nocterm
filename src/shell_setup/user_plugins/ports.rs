//! The ports a user plugin host is composed from, chosen by the mode of the run: files under
//! the state directory for an installation, memory for a developer session.
use std::{path::Path, rc::Rc, sync::Arc};

use gpui_kit::App;
use nocterm_plugin_host::{HostPorts, SessionData, SessionRecords};
use nocterm_plugin_model::PluginCatalog;
use nocterm_plugin_packages::{PackageStore, SessionStore};
use nocterm_store::ShutdownCoordinator;

use super::{
    PluginDataFiles, RecordsFile, reserved_names,
    runtime::{TracingGuestLog, runtimes},
};

/// The ports of an installation: the records file, one data file per plugin and one package
/// file per plugin under `root`, each written through the shared persistence service.
pub(super) fn installed(
    root: &Path,
    catalog: &PluginCatalog,
    shutdown: &ShutdownCoordinator,
    cx: &mut App,
) -> HostPorts {
    let records = RecordsFile::open(root.join("records.json"), cx);
    let data = PluginDataFiles::new(root.join("data"), cx);
    for (owner, registered) in [
        ("plugin records", records.register_shutdown(shutdown)),
        ("plugin data", data.register_shutdown(shutdown)),
    ] {
        if let Err(error) = registered {
            tracing::error!(owner, %error, "shutdown registration failed");
        }
    }
    HostPorts {
        records: Rc::new(records),
        data: Rc::new(data),
        packages: Arc::new(PackageStore::new(
            root.join("packages"),
            reserved_names(catalog),
        )),
        runtime: runtimes(Arc::new(TracingGuestLog::installed())),
    }
}

/// The ports of a developer session: everything lives in memory and nothing is registered for
/// shutdown, so the session can leave no file behind.
pub(super) fn session(catalog: &PluginCatalog) -> HostPorts {
    HostPorts {
        records: Rc::new(SessionRecords::new()),
        data: Rc::new(SessionData::new()),
        packages: Arc::new(SessionStore::new(reserved_names(catalog))),
        runtime: runtimes(Arc::new(TracingGuestLog::dev())),
    }
}
