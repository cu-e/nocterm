//! Plugin data and records survive a restart, and a bad file never takes the host down.
use std::path::Path;

use gpui_kit::TestAppContext;
use nocterm_plugin_api::capability::storage::Set;
use nocterm_plugin_host::{
    AuditAction, PluginData, PluginDataChange, PluginDataDoc, PluginDataError, PluginRecord,
    PluginRecords, RecordsDoc, RecordsError,
};
use nocterm_plugin_model::PluginId;
use nocterm_store::ShutdownCoordinator;

use super::{PluginDataFiles, RecordsFile};

fn plugin() -> PluginId {
    PluginId::user("acme", "clock").expect("a valid plugin id")
}

fn doc(key: &str, value: &str) -> PluginDataDoc {
    PluginDataDoc::default()
        .with_set(&Set {
            key: key.into(),
            value: value.into(),
        })
        .expect("inside the bounds")
}

fn files(root: &Path, cx: &mut TestAppContext) -> PluginDataFiles {
    cx.update(|cx| PluginDataFiles::new(root.to_owned(), cx))
}

fn load(
    files: &PluginDataFiles,
    id: &PluginId,
    cx: &mut TestAppContext,
) -> Result<PluginDataDoc, PluginDataError> {
    cx.update(|cx| files.load(id, cx))
}

fn change(
    files: &PluginDataFiles,
    id: &PluginId,
    change: PluginDataChange,
    cx: &mut TestAppContext,
) {
    cx.update(|cx| files.change(id.clone(), change, cx))
        .expect("the change is admitted");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_plugin_without_a_file_has_default_data(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let data = files(root.path(), cx);

    assert_eq!(load(&data, &plugin(), cx), Ok(PluginDataDoc::default()));
}

#[gpui_kit::test]
fn a_change_reaches_a_fresh_instance(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let data = files(root.path(), cx);
    change(
        &data,
        &plugin(),
        PluginDataChange::Replace(doc("city", "Oslo")),
        cx,
    );

    let restarted = files(root.path(), cx);
    assert_eq!(load(&restarted, &plugin(), cx), Ok(doc("city", "Oslo")));
}

#[gpui_kit::test]
fn the_latest_change_to_a_plugin_wins(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let data = files(root.path(), cx);
    cx.update(|cx| {
        for value in ["a", "b", "c"] {
            data.change(plugin(), PluginDataChange::Replace(doc("k", value)), cx)
                .expect("admitted");
        }
    });
    cx.run_until_parked();

    assert_eq!(load(&data, &plugin(), cx), Ok(doc("k", "c")));
}

#[gpui_kit::test]
fn removing_a_plugin_deletes_its_file(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let data = files(root.path(), cx);
    change(
        &data,
        &plugin(),
        PluginDataChange::Replace(doc("k", "v")),
        cx,
    );
    assert!(root.path().join("user-acme-clock.json").exists());

    change(&data, &plugin(), PluginDataChange::Remove, cx);

    assert!(!root.path().join("user-acme-clock.json").exists());
    assert_eq!(load(&data, &plugin(), cx), Ok(PluginDataDoc::default()));
}

#[gpui_kit::test]
fn a_broken_file_is_reported_and_replaced_by_the_next_change(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("user-acme-clock.json");
    std::fs::write(&file, b"{ not json").unwrap();
    let data = files(root.path(), cx);

    assert!(matches!(
        load(&data, &plugin(), cx),
        Err(PluginDataError::Broken(_))
    ));
    assert_eq!(std::fs::read(&file).unwrap(), b"{ not json");

    change(
        &data,
        &plugin(),
        PluginDataChange::Replace(doc("k", "v")),
        cx,
    );
    assert_eq!(load(&data, &plugin(), cx), Ok(doc("k", "v")));
}

#[gpui_kit::test]
fn an_oversized_file_is_broken_without_being_read_whole(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("user-acme-clock.json"),
        vec![b' '; 600 * 1024],
    )
    .unwrap();
    let data = files(root.path(), cx);

    assert!(matches!(
        load(&data, &plugin(), cx),
        Err(PluginDataError::Broken(_))
    ));
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_linked_file_is_not_followed(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let outside = root.path().join("outside.json");
    std::fs::write(&outside, b"{}").unwrap();
    let plugins = root.path().join("plugins");
    std::fs::create_dir(&plugins).unwrap();
    std::os::unix::fs::symlink(&outside, plugins.join("user-acme-clock.json")).unwrap();
    let data = files(&plugins, cx);

    assert!(matches!(
        load(&data, &plugin(), cx),
        Err(PluginDataError::Broken(_))
    ));
}

#[gpui_kit::test]
fn an_id_that_is_not_a_plugin_id_names_no_file(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let data = files(root.path(), cx);
    let escaping = PluginId::owned("../escape");

    assert!(matches!(
        load(&data, &escaping, cx),
        Err(PluginDataError::NotAPluginId(_))
    ));
    let refused = cx.update(|cx| {
        data.change(
            escaping.clone(),
            PluginDataChange::Replace(doc("k", "v")),
            cx,
        )
    });
    cx.run_until_parked();
    assert!(matches!(refused, Err(PluginDataError::NotAPluginId(_))));
    assert!(std::fs::read_dir(root.path()).unwrap().next().is_none());
    assert!(!root.path().parent().unwrap().join("escape.json").exists());
}

#[gpui_kit::test]
fn shutdown_flushes_what_is_still_pending(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let data = files(root.path(), cx);
    let shutdown = ShutdownCoordinator::default();
    data.register_shutdown(&shutdown).unwrap();
    cx.update(|cx| data.change(plugin(), PluginDataChange::Replace(doc("k", "v")), cx))
        .expect("admitted");

    let report = cx.update(|cx| shutdown.capture_all(cx));
    let report = cx.foreground_executor().block_test(report);
    assert!(report.failures().is_empty() && !report.timed_out());

    let restarted = files(root.path(), cx);
    assert_eq!(load(&restarted, &plugin(), cx), Ok(doc("k", "v")));
}

#[gpui_kit::test]
fn a_change_after_shutdown_began_is_refused_to_the_caller(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let data = files(root.path(), cx);
    let shutdown = ShutdownCoordinator::default();
    data.register_shutdown(&shutdown).unwrap();
    let report = cx.update(|cx| shutdown.capture_all(cx));
    cx.foreground_executor().block_test(report);

    let refused =
        cx.update(|cx| data.change(plugin(), PluginDataChange::Replace(doc("k", "v")), cx));

    assert!(
        matches!(refused, Err(PluginDataError::NotSaved(_))),
        "a change that will never be written is not reported as saved: {refused:?}"
    );
    assert!(!root.path().join("user-acme-clock.json").exists());
}

#[test]
fn the_worst_case_document_fits_the_file_cap() {
    use nocterm_plugin_api::capability::storage::{MAX_TOTAL, MAX_VALUE};

    // Control characters take six bytes each once JSON escapes them.
    let escaped = "\u{1}".repeat(MAX_VALUE / 4);
    let mut worst = PluginDataDoc::default();
    for index in 0..=MAX_TOTAL / MAX_VALUE {
        let key = format!("k{index:02}");
        match worst.with_set(&Set {
            key,
            value: escaped.clone(),
        }) {
            Ok(next) => worst = next,
            Err(_) => break,
        }
    }
    let encoded = worst.encode().expect("a bounded document encodes");

    assert!(
        encoded.len() <= super::data::MAX_FILE_BYTES,
        "{} bytes do not fit a cap of {}",
        encoded.len(),
        super::data::MAX_FILE_BYTES
    );
    assert!(PluginDataDoc::decode(&encoded).is_ok());
}

fn record() -> PluginRecord {
    PluginRecord {
        id: plugin(),
        hash: "ab".repeat(32),
        enabled: true,
        placeable: false,
        granted_optional: Vec::new(),
    }
}

fn records_doc() -> RecordsDoc {
    RecordsDoc {
        plugins: vec![record()],
        audit: vec![nocterm_plugin_host::AuditEntry {
            seq: 0,
            plugin: plugin(),
            action: AuditAction::Installed,
        }],
    }
}

fn open_records(file: &Path, cx: &mut TestAppContext) -> RecordsFile {
    cx.update(|cx| RecordsFile::open(file.to_owned(), cx))
}

#[gpui_kit::test]
fn records_are_empty_without_a_file(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let records = open_records(&root.path().join("plugin-records.json"), cx);

    assert_eq!(cx.update(|cx| records.load(cx)), RecordsDoc::default());
}

#[gpui_kit::test]
fn replaced_records_are_published_and_survive_a_restart(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("plugin-records.json");
    let records = open_records(&file, cx);

    let task = cx.update(|cx| records.replace(records_doc(), cx));
    cx.foreground_executor().block_test(task).unwrap();

    assert_eq!(cx.update(|cx| records.load(cx)), records_doc());
    let restarted = open_records(&file, cx);
    assert_eq!(cx.update(|cx| restarted.load(cx)), records_doc());
}

#[gpui_kit::test]
fn a_broken_records_file_starts_empty_and_is_replaced(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("plugin-records.json");
    std::fs::write(&file, b"][").unwrap();
    let records = open_records(&file, cx);
    assert_eq!(cx.update(|cx| records.load(cx)), RecordsDoc::default());

    let task = cx.update(|cx| records.replace(records_doc(), cx));
    cx.foreground_executor().block_test(task).unwrap();

    let restarted = open_records(&file, cx);
    assert_eq!(cx.update(|cx| restarted.load(cx)), records_doc());
}

#[gpui_kit::test]
fn a_failed_records_write_is_reported(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    // A directory where the file belongs cannot be published over.
    let file = root.path().join("plugin-records.json");
    std::fs::create_dir(&file).unwrap();
    std::fs::write(file.join("keep"), b"x").unwrap();
    let records = open_records(&file, cx);

    let task = cx.update(|cx| records.replace(records_doc(), cx));

    let Err(RecordsError::Write(cause)) = cx.foreground_executor().block_test(task) else {
        panic!("a failed write is its own error");
    };
    let mut chain = vec![cause.to_string()];
    let mut source = std::error::Error::source(&cause);
    while let Some(error) = source {
        chain.push(error.to_string());
        source = error.source();
    }
    assert!(chain.len() > 1, "the cause keeps its own causes: {chain:?}");
}

#[gpui_kit::test]
fn records_flush_at_shutdown(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("plugin-records.json");
    let records = open_records(&file, cx);
    let shutdown = ShutdownCoordinator::default();
    records.register_shutdown(&shutdown).unwrap();
    drop(cx.update(|cx| records.replace(records_doc(), cx)));

    let report = cx.update(|cx| shutdown.capture_all(cx));
    let report = cx.foreground_executor().block_test(report);
    assert!(report.failures().is_empty());

    let restarted = open_records(&file, cx);
    assert_eq!(cx.update(|cx| restarted.load(cx)), records_doc());
}

mod window {
    use std::sync::Arc;

    use gpui_kit::{AppContext as _, TestAppContext, WindowOptions};
    use nocterm_files::TransferCounts;
    use nocterm_monitor_ui::MonitorUsage;
    use nocterm_plugin_api::{
        Capability as _, Effect, HostError, Observed,
        capability::{
            notifications::{NoticeTone, Post},
            transfers::{Counts, CountsNow},
            workspace::{Kind, WorkspaceKind, WorkspaceState},
        },
    };
    use nocterm_transfers::testing::FakeTransfers;
    use nocterm_workspace::Workspace;

    use super::{super::window_providers, plugin};

    #[gpui_kit::test]
    fn a_window_answers_what_its_own_ui_shows_and_nothing_more(cx: &mut TestAppContext) {
        let (handle, workspace) = cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(nocterm_ui::Design::new(nocterm_ui::DesignTokens::builtin()));
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let layout = nocterm_ui::LayoutHandle::in_memory(cx);
                    Workspace::new(nocterm_workspace::Deps::new(layout), window, cx)
                })
            })
            .expect("a window opens")
        });
        let providers = cx.update(|cx| {
            window_providers(
                &workspace,
                handle,
                &MonitorUsage::new(),
                TransferCounts::over(Arc::new(FakeTransfers::default())),
                cx,
            )
        });

        cx.update(|cx| {
            assert_eq!(
                providers.read(Kind::ID, &plugin(), cx),
                Ok(Observed::Workspace(WorkspaceState {
                    kind: WorkspaceKind::Local,
                    connected: true
                }))
            );
            assert_eq!(
                providers.read(CountsNow::ID, &plugin(), cx),
                Ok(Observed::Counts(Counts {
                    active: 0,
                    failed: 0
                }))
            );
            // No monitor widget was created, so no reading exists and none is started.
            assert_eq!(
                providers.read("monitor.usage", &plugin(), cx),
                Err(HostError::Unavailable)
            );
            providers
                .perform(
                    &plugin(),
                    Effect::NotificationsPost(Post {
                        tone: NoticeTone::Info,
                        text: "hello".into(),
                        notice_key: None,
                    }),
                    cx,
                )
                .expect("the window is open");
        });
        cx.update(|cx| {
            let Ok(Observed::Notices(own)) = providers.read("notifications.own", &plugin(), cx)
            else {
                panic!("the window answers its notices");
            };
            assert_eq!(own.0.len(), 1);
        });
    }
}
