use super::*;
use crate::vault_keys::VaultKeys;
use futures::executor::block_on;
use nocterm_ssh_keys::VaultSshKeys as _;
use nocterm_vault::testing::ScriptedVaultAccess;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

/// One poll of `future`, which must not be left waiting on anything this test cannot wake.
fn poll_once<T>(future: &mut (impl Future<Output = T> + Unpin)) -> Poll<T> {
    Pin::new(future).poll(&mut Context::from_waker(Waker::noop()))
}

fn unlocked() -> Arc<ScriptedVaultAccess> {
    let vault = Arc::new(ScriptedVaultAccess::default());
    block_on(vault.create(Secret::new("test unique master passphrase"))).unwrap();
    vault
}

#[test]
fn calls_reach_the_vault_selected_last() {
    let own = unlocked();
    let account = unlocked();
    let selected = SelectedVault::new(own.clone());
    block_on(selected.put(
        None,
        "own".into(),
        CredentialBinding::KeyPassphrase { path: "key".into() },
        Secret::new("one"),
    ))
    .unwrap();
    selected.select(account.clone());
    block_on(account.unlock(Secret::new("test unique master passphrase"))).unwrap();
    assert!(block_on(selected.list()).unwrap().is_empty());
    assert_eq!(block_on(own.list()).unwrap().len(), 1);
}

#[test]
fn selecting_another_vault_locks_the_one_it_replaces() {
    let own = unlocked();
    let selected = SelectedVault::new(own.clone());
    selected.select(Arc::new(ScriptedVaultAccess::default()));
    assert_eq!(own.status(), VaultStatus::Locked);
    assert_eq!(selected.status(), VaultStatus::Missing);
}

#[test]
fn selecting_the_vault_in_use_changes_nothing() {
    let own = unlocked();
    let selected = SelectedVault::new(own.clone());
    let revision = selected.revision();
    selected.select(own.clone());
    assert!(own.is_unlocked());
    assert_eq!(selected.revision(), revision);
}

#[test]
fn the_revision_rises_across_a_switch_even_to_a_fresh_vault() {
    let selected = SelectedVault::new(unlocked());
    let before = selected.revision();
    selected.select(Arc::new(ScriptedVaultAccess::default()));
    let after = selected.revision();
    assert!(after > before);
    selected.select(unlocked());
    assert!(selected.revision() > after);
}

#[test]
fn an_observer_is_woken_by_a_switch_and_by_a_change_of_the_selected_vault() {
    let own = unlocked();
    let selected = SelectedVault::new(own.clone());
    let seen = selected.revision();
    let mut waiting = selected.changed(seen);
    assert!(poll_once(&mut waiting).is_pending());
    selected.select(Arc::new(ScriptedVaultAccess::default()));
    let woken = block_on(waiting);
    assert_eq!(woken, selected.revision());

    let account = Arc::new(ScriptedVaultAccess::default());
    selected.select(account.clone());
    let seen = selected.revision();
    let mut waiting = selected.changed(seen);
    assert!(poll_once(&mut waiting).is_pending());
    block_on(account.create(Secret::new("test unique master passphrase"))).unwrap();
    assert!(block_on(waiting) > seen);
}

#[test]
fn an_observer_with_an_old_revision_is_answered_at_once() {
    let selected = SelectedVault::new(unlocked());
    let seen = selected.revision();
    selected.select(unlocked());
    assert_eq!(
        poll_once(&mut selected.changed(seen)),
        Poll::Ready(selected.revision())
    );
}

#[test]
fn waiting_for_an_unlock_follows_the_switch() {
    let selected = SelectedVault::new(Arc::new(ScriptedVaultAccess::default()));
    let mut waiting = selected.wait_unlocked();
    assert!(poll_once(&mut waiting).is_pending());
    selected.select(unlocked());
    block_on(waiting);
}

#[test]
fn the_auto_lock_applies_to_the_vault_selected_later() {
    let selected = SelectedVault::new(unlocked());
    let account = Arc::new(ScriptedVaultAccess::default());
    selected.set_auto_lock(Duration::from_secs(60));
    selected.select(account.clone());
    assert_eq!(account.auto_lock_updates(), [Duration::from_secs(60)]);
}

#[gpui_kit::test]
fn the_keys_of_the_connection_editor_follow_the_selected_vault(cx: &mut gpui_kit::TestAppContext) {
    let own = unlocked();
    let selected = Arc::new(SelectedVault::new(own));
    let keys = VaultKeys::new(selected.clone(), cx.executor().clone());
    let executor = cx.foreground_executor().clone();
    executor.block_test(keys.generate("laptop".into())).unwrap();
    assert_eq!(executor.block_test(keys.list()).unwrap().len(), 1);
    let account = Arc::new(ScriptedVaultAccess::default());
    selected.select(account.clone());
    block_on(account.create(Secret::new("test unique master passphrase"))).unwrap();
    assert!(executor.block_test(keys.list()).unwrap().is_empty());
}

#[test]
fn going_home_selects_the_vault_of_this_device_again() {
    let own = unlocked();
    let selected = SelectedVault::new(own.clone());
    selected.select(Arc::new(ScriptedVaultAccess::default()));
    selected.select_home();
    assert_eq!(selected.status(), own.status());
    assert_eq!(own.status(), VaultStatus::Locked);
    block_on(selected.unlock(Secret::new("test unique master passphrase"))).unwrap();
    assert!(own.is_unlocked());
}
