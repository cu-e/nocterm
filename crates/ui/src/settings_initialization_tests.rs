use crate::TerminalSettings;
use gpui_kit::Task;
use gpui_kit::TestAppContext;
use nocterm_settings::{SettingsDocument, SettingsFile};
use nocterm_store::ShutdownCoordinator;
use nocterm_store::{SettingsConfig, SettingsHandle, StoreError};

fn install(config: SettingsConfig, cx: &mut TestAppContext) -> SettingsHandle {
    cx.update(|cx| {
        gpui_kit::init(cx);
        let ready = crate::init(crate::DesignTokens::builtin(), config.install(cx), cx);
        let settings = ready.settings().clone();
        let shutdown = ShutdownCoordinator::default();
        settings.register_shutdown(&shutdown).unwrap();
        crate::LayoutMemory::init(None, &shutdown, cx);
        settings
    })
}

fn on_disk(file: &SettingsFile) -> TerminalSettings {
    let mut document = file.load().unwrap();
    document.register::<TerminalSettings>();
    document.get::<TerminalSettings>().clone()
}

fn copy_on_select(
    settings: &SettingsHandle,
    cx: &mut TestAppContext,
) -> Task<Result<u64, StoreError>> {
    cx.update(|cx| {
        settings.update::<TerminalSettings>(cx, |terminal| terminal.copy_on_select = true)
    })
}

#[gpui_kit::test]
async fn installed_service_handle_keeps_its_identity_through_ui_access(cx: &mut TestAppContext) {
    let handle = cx.update(|cx| {
        SettingsConfig::in_memory(SettingsDocument::default())
            .with(TerminalSettings::default())
            .install(cx)
    });
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::init(crate::DesignTokens::builtin(), handle.clone(), cx);
    });
    let settings = handle.clone();
    assert_eq!(copy_on_select(&settings, cx).await.unwrap(), 1);
    cx.update(|cx| {
        assert!(handle.setting::<TerminalSettings>(cx).copy_on_select);
        assert_eq!(handle.revision(cx), 1);
    });
}

#[gpui_kit::test]
async fn quitting_with_queued_writes_does_not_panic(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = SettingsFile::new(directory.path().join("settings.toml"));
    let settings = install(
        SettingsConfig::new(SettingsDocument::default(), file.clone()),
        cx,
    );
    let _save = copy_on_select(&settings, cx);
    cx.quit();
    assert!(
        on_disk(&file).copy_on_select,
        "accepted edits must reach disk during actual GPUI shutdown"
    );
}

#[gpui_kit::test]
fn quit_flushes_accepted_settings_before_foreground_writer_runs(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = SettingsFile::new(directory.path().join("settings.toml"));
    let settings = install(SettingsConfig::new(Default::default(), file.clone()), cx);
    let pending = cx.update(|cx| {
        let first = settings.update::<TerminalSettings>(cx, |s| s.font_size = Some(23.));
        let second = settings.update::<TerminalSettings>(cx, |s| s.copy_on_select = true);
        (first, second)
    });
    cx.quit();
    let saved = on_disk(&file);
    assert_eq!(saved.font_size, Some(23.));
    assert!(saved.copy_on_select);
    drop(pending);
}
