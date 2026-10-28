//! Opening a hub in a window of its own or in the window that asks, through the port of the
//! environment menu, and what the opened window can do at once.
use super::hub_account::{ADDRESS, Account, FakeAccounts, PASSPHRASE, World, home, hub_id, world};
use super::*;
use crate::windows::AppEnvironmentOpener;
use nocterm_connections_model::replica::ReplicaScope;
use nocterm_hubs_model::Environment;
use nocterm_hubs_ui::{EnvironmentOpener as _, Placement};
use nocterm_vault::{VaultAccess as _, VaultStatus};

/// A locked account with one connection, and the application over it.
fn locked_account(
    cx: &mut TestAppContext,
) -> (World, Arc<nocterm_vault::testing::ScriptedVaultAccess>) {
    let account = Account::new();
    account.replica.write_remote(
        ReplicaScope::Personal,
        super::hub_account::profile("remote"),
    );
    account.vault.lock();
    account.replica.set_locked(true);
    let vault = account.vault.clone();
    (
        world(FakeAccounts::of(vec![(hub_id(ADDRESS), account)]), cx),
        vault,
    )
}

/// Asks the application, from `handle`'s window, to open `environment`.
fn ask(
    world: &World,
    handle: AnyWindowHandle,
    environment: Environment,
    placement: Placement,
    cx: &mut TestAppContext,
) {
    let opener = AppEnvironmentOpener::new(world.booted.clone());
    cx.update_window(handle, |_, window, cx| {
        opener
            .open(environment, placement, window, cx)
            .expect("the environment opens");
    })
    .unwrap();
    cx.run_until_parked();
}

fn unlock_prompt_is_shown(handle: AnyWindowHandle, cx: &mut TestAppContext) -> bool {
    // The window asks for the vault a couple of frames after it is composed.
    deliver_frames(handle, 4, cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find("vault-unlock-prompt").is_some()
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_server_chosen_in_the_menu_replaces_the_window_and_asks_for_its_vault(cx: &mut TestAppContext) {
    let (world, vault) = locked_account(cx);
    let (handle, _workspace) = world.open(&Environment::Local, cx);
    assert!(!unlock_prompt_is_shown(handle, cx));

    ask(&world, handle, home(), Placement::ThisWindow, cx);

    assert_eq!(cx.windows().len(), 1, "the window is kept");
    assert!(
        unlock_prompt_is_shown(handle, cx),
        "the unlock reaches the vault of the hub in the switched window"
    );
    cx.simulate_input(handle, PASSPHRASE);
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(vault.is_unlocked());
    assert_eq!(
        world.vault_of(&Environment::Local, cx).status(),
        VaultStatus::Missing,
        "the vault of this computer was not asked"
    );
}

#[gpui_kit::test]
fn a_server_opened_in_a_new_window_leaves_the_asking_window_alone(cx: &mut TestAppContext) {
    let (world, _vault) = locked_account(cx);
    let (handle, _workspace) = world.open(&Environment::Local, cx);

    ask(&world, handle, home(), Placement::NewWindow, cx);

    assert_eq!(cx.windows().len(), 2);
    assert!(
        !unlock_prompt_is_shown(handle, cx),
        "the window that asked still shows this computer"
    );
    let hub_window = cx
        .windows()
        .into_iter()
        .find(|window| *window != handle)
        .unwrap();
    assert!(unlock_prompt_is_shown(hub_window, cx));
}

#[gpui_kit::test]
fn a_server_that_is_not_listed_is_refused_and_no_window_changes(cx: &mut TestAppContext) {
    let (world, _vault) = locked_account(cx);
    let (handle, _workspace) = world.open(&Environment::Local, cx);
    let opener = AppEnvironmentOpener::new(world.booted.clone());
    let unknown = Environment::Hub(hub_id("stranger.example:7443"));

    let refused = cx
        .update_window(handle, |_, window, cx| {
            opener
                .open(unknown, Placement::NewWindow, window, cx)
                .is_err()
        })
        .unwrap();
    cx.run_until_parked();

    assert!(refused);
    assert_eq!(cx.windows().len(), 1);
}
