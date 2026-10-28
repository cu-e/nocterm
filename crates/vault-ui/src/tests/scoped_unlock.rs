//! `UnlockVault` and `LockVault` act on the vault of the window they are sent in.
use futures::executor::block_on;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Focusable as _, TestAppContext, WindowOptions,
    test::TestWindowExt as _,
};
use nocterm_session::Secret;
use nocterm_vault::{VaultAccess, testing::ScriptedVaultAccess};
use nocterm_workspace::{LockVault, UnlockVault, Workspace, test_support::deps_for};
use std::sync::Arc;

const PASSWORD: &str = "master password";

/// An application vault, a scoped one, and a window for each way of
/// reaching them: `scoped` registered the scoped vault, `plain` did not.
struct Windows {
    app: Arc<ScriptedVaultAccess>,
    scoped_vault: Arc<ScriptedVaultAccess>,
    scoped: AnyWindowHandle,
    plain: AnyWindowHandle,
}

fn locked_vault() -> Arc<ScriptedVaultAccess> {
    let vault = Arc::new(ScriptedVaultAccess::default());
    block_on(vault.create(Secret::new(PASSWORD))).unwrap();
    vault.lock();
    vault
}

fn open_workspace(scoped: Option<&crate::Installed>, cx: &mut gpui_kit::App) -> AnyWindowHandle {
    let (handle, workspace) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
        cx.new(|cx| {
            let mut workspace = Workspace::new(deps_for(&[], cx), window, cx);
            if let Some(installed) = scoped {
                crate::register_scoped(installed, &mut workspace);
            }
            workspace
        })
    })
    .unwrap();
    // Actions reach a workspace's handlers through focus inside it, as in a used window.
    handle
        .update(cx, |_, window, cx| {
            window.focus(&workspace.read(cx).focus_handle(cx), cx)
        })
        .unwrap();
    handle
}

fn windows(cx: &mut TestAppContext) -> Windows {
    let app = locked_vault();
    let scoped_vault = locked_vault();
    let (scoped, plain) = cx.update(|cx| {
        gpui_kit::init(cx);
        // The unlock dialog slides in on wall-clock time; see `setup_with_store`.
        cx.set_reduce_motion(true);
        let ready = nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx),
            cx,
        );
        crate::install(crate::Deps::new(app.clone()), &ready, cx);
        let installed = crate::install_scoped(crate::Deps::new(scoped_vault.clone()), &ready, cx);
        (
            open_workspace(Some(&installed), cx),
            open_workspace(None, cx),
        )
    });
    Windows {
        app,
        scoped_vault,
        scoped,
        plain,
    }
}

/// Sends `UnlockVault` inside `window`, then types the master password.
fn unlock_in(window: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(window, |_, window, cx| {
        window.activate_window();
        window.dispatch_action(Box::new(UnlockVault), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("vault-unlock-prompt").is_some(),
            "the unlock dialog is open in the window the action was sent in"
        );
    })
    .unwrap();
    cx.simulate_input(window, PASSWORD);
    cx.update_window(window, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn unlock_in_a_window_with_a_scoped_vault_opens_that_vaults_dialog(cx: &mut TestAppContext) {
    let setup = windows(cx);
    unlock_in(setup.scoped, cx);
    assert!(setup.scoped_vault.is_unlocked());
    assert!(!setup.app.is_unlocked());
}

#[gpui_kit::test]
fn unlock_in_a_window_without_a_scoped_vault_opens_the_applications_dialog(
    cx: &mut TestAppContext,
) {
    let setup = windows(cx);
    unlock_in(setup.plain, cx);
    assert!(setup.app.is_unlocked());
    assert!(!setup.scoped_vault.is_unlocked());
}

#[gpui_kit::test]
fn lock_in_a_window_with_a_scoped_vault_locks_only_that_vault(cx: &mut TestAppContext) {
    let setup = windows(cx);
    block_on(setup.app.unlock(Secret::new(PASSWORD))).unwrap();
    block_on(setup.scoped_vault.unlock(Secret::new(PASSWORD))).unwrap();
    cx.update_window(setup.scoped, |_, window, cx| {
        window.activate_window();
        window.dispatch_action(Box::new(LockVault), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!setup.scoped_vault.is_unlocked());
    assert!(setup.app.is_unlocked());
}
