//! A vault whose keys are not on this device offers the remedy, not a new vault.
use crate::VaultView;
use gpui_kit::{AppContext as _, TestAppContext, WindowOptions, test::TestWindowExt as _};
use nocterm_vault::testing::ScriptedVaultAccess;
use std::{cell::Cell, rc::Rc, sync::Arc};

#[gpui_kit::test]
fn settings_page_offers_to_sign_in_again_instead_of_creating_a_vault(cx: &mut TestAppContext) {
    let calls = Rc::new(Cell::new(0));
    let counted = calls.clone();
    let handle = cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        let ready = nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx),
            cx,
        );
        let service = Arc::new(ScriptedVaultAccess::default());
        let remedy: crate::KeysMissingRemedy = Rc::new(move |_, _| counted.set(counted.get() + 1));
        let installed = crate::install_scoped(
            crate::Deps::new(service.clone()).with_keys_missing(remedy.clone()),
            &ready,
            cx,
        );
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                VaultView::new(service, installed.settings().clone(), window, cx)
                    .with_keys_missing(Some(remedy))
            })
        })
        .unwrap()
        .0
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("vault-password").is_none());
        assert!(window.try_find("vault-submit").is_none());
        window.click("vault-keys-missing", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(calls.get(), 1);
}
