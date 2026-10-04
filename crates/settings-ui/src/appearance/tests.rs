use super::*;
use crate::tests::open;
use gpui_kit::{TestAppContext, component::select::SelectEvent};
use nocterm_themes::{ExtensionInfo, ThemeCatalog, ThemeDirs, ThemeError};
use nocterm_ui::init_themes;
use std::sync::atomic::{AtomicUsize, Ordering};
struct FakeRegistry {
    searches: AtomicUsize,
    downloads: AtomicUsize,
}
fn info(id: &str) -> ExtensionInfo {
    serde_json::from_str(&format!(
        r#"{{"id":"{id}","name":"{id}","version":"1","authors":["Author"],"download_count":42}}"#
    ))
    .unwrap()
}
fn pack() -> Vec<u8> {
    let data=br##"{"themes":[{"name":"Fake Dark","appearance":"dark","style":{"background":"#123"}},{"name":"Fake Light","appearance":"light","style":{"background":"#eee"}}]}"##;
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::fast(),
    ));
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, "themes/fake.json", data.as_slice())
        .unwrap();
    builder.into_inner().unwrap().finish().unwrap()
}
impl ThemeRegistry for FakeRegistry {
    fn search(&self, _: &str) -> Result<Vec<ExtensionInfo>, ThemeError> {
        self.searches.fetch_add(1, Ordering::SeqCst);
        Ok(vec![info("fake"), info("failure")])
    }
    fn download(&self, extension: &ExtensionInfo) -> Result<Vec<u8>, ThemeError> {
        self.downloads.fetch_add(1, Ordering::SeqCst);
        if extension.id == "failure" {
            Err(ThemeError::Invalid("test download failure".into()))
        } else {
            Ok(pack())
        }
    }
}
#[gpui_kit::test]
async fn browser_explicit_actions_install_pick_and_uninstall(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let dirs = ThemeDirs {
        user: root.path().join("user"),
        installed: root.path().join("installed"),
    };
    let fake = Arc::new(FakeRegistry {
        searches: AtomicUsize::new(0),
        downloads: AtomicUsize::new(0),
    });
    // open() initializes globals; inject the client before constructing the page.
    cx.update(|cx| {
        crate::tests::init(cx);
        init_themes(dirs.clone(), ThemeCatalog::load(&dirs), cx);
        init_theme_registry(fake.clone(), cx);
    });
    let (handle, view) = cx.update(|cx| {
        gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| SettingsView::new(window, cx))
        })
        .unwrap()
    });
    let browser = cx.update(|cx| view.read(cx).appearance.browser.clone());
    cx.run_until_parked();
    assert_eq!(fake.searches.load(Ordering::SeqCst), 0);
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 0);
    browser.update(cx, |browser, cx| browser.search(false, true, cx));
    cx.run_until_parked();
    assert_eq!(fake.searches.load(Ordering::SeqCst), 1);
    browser.update(cx, |browser, cx| browser.install(info("fake"), cx));
    cx.update(|cx| assert!(busy("fake", cx)));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!busy("fake", cx));
        assert_eq!(
            choices(Appearance::Dark, cx),
            ["Nocterm Default", "Fake Dark"]
        );
        assert_eq!(
            choices(Appearance::Light, cx),
            ["Nocterm Default", "Fake Light"]
        );
    });
    cx.update_window(handle, |_, _, cx| {
        let picker = view.read(cx).appearance.dark.clone();
        picker.update(cx, |_, cx| {
            cx.emit(SelectEvent::Confirm(Some("Fake Dark".to_owned())))
        });
        let picker = view.read(cx).appearance.light.clone();
        picker.update(cx, |_, cx| {
            cx.emit(SelectEvent::Confirm(Some("Fake Light".to_owned())))
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            cx.settings().appearance.dark_theme.as_deref(),
            Some("Fake Dark")
        )
    });
    browser.update(cx, |browser, cx| browser.install(info("failure"), cx));
    cx.run_until_parked();
    browser.update(cx, |browser, cx| {
        assert!(
            browser
                .errors
                .get("failure")
                .unwrap()
                .contains("test download failure")
        );
        assert!(!browser.errors.contains_key("fake"));
        assert_eq!(browser.status(&info("fake"), cx), ("Installed", true));
    });
    view.update(cx, |view, cx| installed::remove(view, "fake".into(), cx));
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(cx.settings().appearance.dark_theme, None);
        assert_eq!(cx.settings().appearance.light_theme, None);
        assert_eq!(choices(Appearance::Dark, cx), ["Nocterm Default"]);
    });
}
#[gpui_kit::test]
fn plain_settings_without_catalogue_never_fetches(cx: &mut TestAppContext) {
    let (_, view) = open(cx);
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            view.read(cx)
                .appearance
                .dark
                .read(cx)
                .selected_value()
                .unwrap(),
            "Nocterm Default"
        )
    });
}

fn initialized_browser(
    cx: &mut TestAppContext,
    dirs: &ThemeDirs,
    fake: Arc<FakeRegistry>,
) -> (gpui_kit::AnyWindowHandle, Entity<SettingsView>) {
    cx.update(|cx| {
        crate::tests::init(cx);
        init_themes(dirs.clone(), ThemeCatalog::load(dirs), cx);
        init_theme_registry(fake, cx);
        gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| SettingsView::new(window, cx))
        })
        .unwrap()
    })
}

fn temporary_dirs(root: &std::path::Path) -> ThemeDirs {
    ThemeDirs {
        user: root.join("user"),
        installed: root.join("installed"),
    }
}

fn fake_registry() -> Arc<FakeRegistry> {
    Arc::new(FakeRegistry {
        searches: AtomicUsize::new(0),
        downloads: AtomicUsize::new(0),
    })
}

#[gpui_kit::test]
async fn uninstall_preserves_selection_when_a_duplicate_pack_survives(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let dirs = temporary_dirs(root.path());
    for id in ["a", "b"] {
        nocterm_themes::install_archive(&dirs.installed, &info(id), &pack()).unwrap();
    }
    let (_, view) = initialized_browser(cx, &dirs, fake_registry());
    cx.update(|cx| {
        nocterm_ui::edit_settings(cx, |settings| {
            settings.appearance.dark_theme = Some("Fake Dark".into());
            settings.appearance.light_theme = Some("Fake Light".into());
        })
    })
    .await
    .unwrap();
    view.update(cx, |view, cx| installed::remove(view, "a".into(), cx));
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            cx.settings().appearance.dark_theme.as_deref(),
            Some("Fake Dark")
        );
        assert_eq!(
            cx.settings().appearance.light_theme.as_deref(),
            Some("Fake Light")
        );
        assert_eq!(
            choices(Appearance::Dark, cx),
            ["Nocterm Default", "Fake Dark"]
        );
        assert_eq!(cx.themes().unwrap().catalog.packs()[0].id, "b");
        assert!(!busy("a", cx));
    });
}

#[gpui_kit::test]
fn installs_are_shared_across_browsers_and_finish_after_browser_release(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let dirs = temporary_dirs(root.path());
    let fake = fake_registry();
    let (handle, view) = initialized_browser(cx, &dirs, fake.clone());
    let first = cx
        .update_window(handle, |_, window, cx| {
            cx.new(|cx| ThemeBrowser::new(Some(fake.clone()), window, cx))
        })
        .unwrap();
    let surviving = cx.update(|cx| view.read(cx).appearance.browser.clone());
    first.update(cx, |browser, cx| browser.install(info("fake"), cx));
    surviving.update(cx, |browser, cx| {
        assert_eq!(browser.status(&info("fake"), cx), ("Working…", true));
        browser.install(info("fake"), cx);
    });
    let released = first.downgrade();
    drop(first);
    cx.run_until_parked();
    assert!(released.upgrade().is_none());
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 1);
    surviving.update(cx, |browser, cx| {
        assert_eq!(browser.status(&info("fake"), cx), ("Installed", true));
        assert!(!busy("fake", cx));
        let mut updated = info("fake");
        updated.version = "2".into();
        assert_eq!(browser.status(&updated, cx), ("Update", false));
    });
    cx.update(|cx| {
        assert_eq!(
            choices(Appearance::Dark, cx),
            ["Nocterm Default", "Fake Dark"]
        )
    });
}

#[gpui_kit::test]
fn superseded_debounced_searches_do_not_send_extra_requests(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let dirs = temporary_dirs(root.path());
    let fake = fake_registry();
    let (_, view) = initialized_browser(cx, &dirs, fake.clone());
    let browser = cx.update(|cx| view.read(cx).appearance.browser.clone());
    browser.update(cx, |browser, cx| browser.search(true, false, cx));
    cx.run_until_parked();
    assert_eq!(fake.searches.load(Ordering::SeqCst), 0);
    browser.update(cx, |browser, cx| browser.search(true, false, cx));
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(350));
    cx.run_until_parked();
    assert_eq!(fake.searches.load(Ordering::SeqCst), 1);
}
