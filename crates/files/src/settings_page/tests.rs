use super::*;
use gpui_kit::{TestAppContext, test::TestWindowExt as _};

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
    let (page, window) = cx.add_window_view(|window, cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            SettingsStore::in_memory(nocterm_settings::SettingsDocument::default()),
            cx,
        );
        ExplorerPage::new(window, cx)
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
        assert!(page.read(cx).fields.contains_key(&Key::Extensions(0)));
        window.render_frame(cx);
        window.click("explorer-rule-remove-0", cx);
    });
    window.run_until_parked();
    window.update(|_, cx| {
        assert!(
            cx.setting::<crate::ExplorerSettings>()
                .open
                .rules
                .is_empty()
        );
        assert!(!page.read(cx).fields.contains_key(&Key::Extensions(0)));
        assert_eq!(page.read(cx).rules, 0);
    });
}
