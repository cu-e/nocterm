use super::*;
use gpui_kit::{TestAppContext, test::TestWindowExt as _};
use nocterm_ui::{SettingsExt as _, SettingsStore};

#[test]
fn fields_read_and_write_the_settings_they_name() {
    let mut explorer = ExplorerSettings::default();
    Key::Args(Side::Remote)
        .write(&mut explorer, r#"-R "+set nu""#)
        .unwrap();
    assert_eq!(explorer.open.remote.args, ["-R", "+set nu"]);
    assert_eq!(Key::Args(Side::Remote).read(&explorer), r#"-R "+set nu""#);
    Key::Excluded.write(&mut explorer, "a, b").unwrap();
    assert_eq!(explorer.indexing.excluded, ["a", "b"]);
    assert!(Key::MaxRemote.write(&mut explorer, "5").is_err());
    Key::MaxRemote.write(&mut explorer, "50_000").unwrap();
    assert_eq!(explorer.indexing.max_remote_entries, 50_000);
    assert!(Key::Extensions(0).write(&mut explorer, "md").is_err());
    explorer.open.rules.push(OpenRule::default());
    Key::Extensions(0).write(&mut explorer, ".md, txt").unwrap();
    Key::RuleProgram(0, Side::Local)
        .write(&mut explorer, " typora ")
        .unwrap();
    assert_eq!(explorer.open.rules[0].extensions, ["md", "txt"]);
    assert_eq!(explorer.open.rules[0].local.program, "typora");
}

#[gpui_kit::test]
fn presets_and_file_types_save_and_rebuild_their_fields(cx: &mut TestAppContext) {
    let (_page, window) = cx.add_window_view(|window, cx| {
        gpui_kit::init(cx);
        let ready = nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            SettingsStore::in_memory(nocterm_settings::SettingsDocument::default()),
            cx,
        );
        ready.settings().register::<crate::ExplorerSettings>(cx);
        ExplorerPage::new(ready.settings().clone(), window, cx)
    });
    window.update(|window, cx| {
        window.render_frame(cx);
        window.click("explorer-remote-preset-nvim", cx);
        window.click("explorer-rule-add", cx);
    });
    window.run_until_parked();
    window.update(|window, cx| {
        let explorer = cx.setting::<crate::ExplorerSettings>();
        assert_eq!(explorer.open.remote.program, "nvim");
        assert_eq!(explorer.open.rules.len(), 1);
        window.render_frame(cx);
        assert!(window.try_find("explorer-rule-remove-0").is_some());
        window.click("explorer-rule-remove-0", cx);
    });
    window.run_until_parked();
    window.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("explorer-rule-remove-0").is_none());
        assert!(
            cx.setting::<crate::ExplorerSettings>()
                .open
                .rules
                .is_empty()
        );
    });
}

fn settings_window(
    settings: nocterm_store::SettingsHandle,
    cx: &mut App,
) -> (gpui_kit::AnyWindowHandle, Entity<ExplorerPage>) {
    gpui_kit::open_window(Default::default(), cx, |window, cx| {
        cx.new(|cx| ExplorerPage::new(settings, window, cx))
    })
    .unwrap()
}

#[gpui_kit::test]
fn shared_explorer_pages_keep_policy_after_unrelated_owner_and_source_window_close(
    cx: &mut TestAppContext,
) {
    use std::{cell::Cell, rc::Rc};
    let (settings, other, first, first_page, second, second_page) = cx.update(|cx| {
        gpui_kit::init(cx);
        let ready = nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            SettingsStore::in_memory(Default::default()),
            cx,
        );
        let settings = ready.settings().clone();
        settings.register::<ExplorerSettings>(cx);
        let (first, a) = settings_window(settings.clone(), cx);
        let (second, b) = settings_window(settings.clone(), cx);
        let other = nocterm_store::SettingsConfig::in_memory(Default::default())
            .with(ExplorerSettings::default())
            .install(cx);
        cx.set_global(SettingsStore::from_handle(other.clone()));
        (settings, other, first, a, second, b)
    });
    let changes = Rc::new(Cell::new(0));
    let seen = changes.clone();
    let _observation =
        cx.update(|cx| cx.observe(&second_page, move |_, _| seen.set(seen.get() + 1)));
    cx.update_window(first, |_, window, cx| {
        window.render_frame(cx);
        window.click("explorer-remote-preset-nvim", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        changes.get() > 0,
        "the other page observes the actual original owner"
    );
    cx.update_window(second, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("explorer-remote-program").value(), Some("nvim"));
    })
    .unwrap();
    cx.update(|cx| {
        assert_eq!(
            settings.setting::<ExplorerSettings>(cx).open.remote.program,
            "nvim"
        );
        assert_ne!(
            other.setting::<ExplorerSettings>(cx).open.remote.program,
            "nvim"
        );
    });
    let weak = first_page.downgrade();
    cx.update_window(first, |_, window, _| window.remove_window())
        .unwrap();
    drop(first_page);
    cx.run_until_parked();
    assert!(weak.upgrade().is_none());
    let (third, _third_page) = cx.update(|cx| settings_window(settings, cx));
    cx.update_window(third, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("explorer-remote-program").value(), Some("nvim"));
    })
    .unwrap();
}
