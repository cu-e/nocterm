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

/// The installation's records and keyed data owners are registered before asynchronous
/// native acquisition; immutable package versions retain their last durable consent.
pub(super) fn installed(
    root: &Path,
    catalog: &PluginCatalog,
    shutdown: &ShutdownCoordinator,
    cx: &mut App,
) -> (HostPorts, Rc<RecordsFile>, Rc<PluginDataFiles>) {
    let records = Rc::new(RecordsFile::new(root.join("records.json"), cx));
    let data = Rc::new(PluginDataFiles::new(root.join("data"), cx));
    for (owner, registered) in [
        ("plugin records", records.register_shutdown(shutdown)),
        ("plugin data", data.register_shutdown(shutdown)),
    ] {
        if let Err(error) = registered {
            tracing::error!(owner, %error, "shutdown registration failed");
        }
    }
    let ports = HostPorts {
        records: records.clone(),
        data: data.clone(),
        packages: Arc::new(PackageStore::new(
            root.join("packages"),
            reserved_names(catalog),
        )),
        runtime: runtimes(Arc::new(TracingGuestLog::installed())),
    };
    (ports, records, data)
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
