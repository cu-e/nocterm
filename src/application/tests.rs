//! Directory actions retain the supplied paths and published settings owner.
use super::*;
use gpui_kit::TestAppContext;
use nocterm_session::LoggingOptions;
use nocterm_settings::SettingsDocument;
use nocterm_store::{SettingsConfig, SnapshotWriter, StoreError};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn directory_actions(
    fail: Arc<AtomicBool>,
    seen: Rc<RefCell<Vec<PathBuf>>>,
    cx: &mut TestAppContext,
) -> SettingsHandle {
    let gate = fail.clone();
    let writer: SnapshotWriter<SettingsDocument> = Arc::new(move |_| {
        let failed = gate.load(Ordering::SeqCst);
        Box::pin(async move {
            if failed {
                Err(StoreError::backend(std::io::Error::from(
                    std::io::ErrorKind::PermissionDenied,
                )))
            } else {
                Ok(())
            }
        })
    });

    let records = seen.clone();
    cx.update(|cx| {
        let settings = SettingsConfig::with_writer(SettingsDocument::default(), writer)
            .with(LoggingOptions::default())
            .install(cx);
        register_directory_actions(
            "/captured/config".into(),
            "/captured/state/logs".into(),
            settings.clone(),
            move |path, _| records.borrow_mut().push(path),
            cx,
        );
        cx.dispatch_action(&ProfilesDirectory);
        cx.dispatch_action(&LogsDirectory);
        settings
    })
}

#[gpui_kit::test]
async fn directory_actions_read_only_the_captured_published_settings_at_each_click(
    cx: &mut TestAppContext,
) {
    let fail = Arc::new(AtomicBool::new(false));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let settings = directory_actions(fail.clone(), seen.clone(), cx);
    assert_eq!(
        *seen.borrow(),
        [
            PathBuf::from("/captured/config"),
            PathBuf::from("/captured/state/logs")
        ]
    );
    cx.update(|cx| {
        settings.update::<LoggingOptions>(cx, |s| s.directory = Some("/published/logs".into()))
    })
    .await
    .unwrap();
    cx.update(|cx| cx.dispatch_action(&LogsDirectory));
    assert_eq!(
        seen.borrow().last().unwrap(),
        &PathBuf::from("/published/logs")
    );
    fail.store(true, Ordering::SeqCst);
    assert!(
        cx.update(|cx| settings
            .update::<LoggingOptions>(cx, |s| s.directory = Some("/rejected/logs".into())))
            .await
            .is_err()
    );
    cx.update(|cx| {
        let _unrelated = SettingsConfig::in_memory(SettingsDocument::default())
            .with(LoggingOptions {
                directory: Some("/other/owner".into()),
                ..LoggingOptions::default()
            })
            .install(cx);
        cx.dispatch_action(&LogsDirectory);
    });
    assert_eq!(
        seen.borrow().last().unwrap(),
        &PathBuf::from("/published/logs")
    );
    fail.store(false, Ordering::SeqCst);
    cx.update(|cx| settings.update::<LoggingOptions>(cx, |s| s.directory = Some(PathBuf::new())))
        .await
        .unwrap();
    cx.update(|cx| cx.dispatch_action(&LogsDirectory));
    assert_eq!(
        seen.borrow().last().unwrap(),
        &PathBuf::new(),
        "Some(empty) is not fallback"
    );
    cx.update(|cx| settings.update::<LoggingOptions>(cx, |s| s.directory = None))
        .await
        .unwrap();
    cx.update(|cx| cx.dispatch_action(&LogsDirectory));
    assert_eq!(
        seen.borrow().last().unwrap(),
        &PathBuf::from("/captured/state/logs")
    );
}

#[gpui_kit::test]
async fn captured_directory_actions_survive_source_window_and_new_window(cx: &mut TestAppContext) {
    use gpui_kit::{AppContext as _, EmptyView, WindowOptions};
    let seen = Rc::new(RefCell::new(Vec::new()));
    let settings = directory_actions(Arc::new(AtomicBool::new(false)), seen.clone(), cx);
    let (first, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| EmptyView)).unwrap()
    });
    let weak = view.downgrade();
    cx.update_window(first, |_, window, cx| {
        window.activate_window();
        window.dispatch_action(Box::new(ProfilesDirectory), cx);
        window.dispatch_action(Box::new(LogsDirectory), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(first, |_, window, _| window.remove_window())
        .unwrap();
    drop(view);
    cx.run_until_parked();
    assert!(weak.upgrade().is_none());
    cx.update(|cx| {
        settings.update::<LoggingOptions>(cx, |s| s.directory = Some("/after/close".into()))
    })
    .await
    .unwrap();
    let (second, _view) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| EmptyView)).unwrap()
    });
    cx.update_window(second, |_, window, cx| {
        window.activate_window();
        window.dispatch_action(Box::new(ProfilesDirectory), cx);
        window.dispatch_action(Box::new(LogsDirectory), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(seen.borrow().ends_with(&[
        PathBuf::from("/captured/config"),
        PathBuf::from("/after/close")
    ]));
}
