use super::*;
use crate::account::testing::NativeAccounts;
use crate::store::memory::StoreBoundary;

fn captured(fixture: &NativeAccounts) -> HubProfileReplica {
    HubProfileReplica::new(
        Handle::current(),
        fixture.operations().bind(fixture.source()).unwrap(),
    )
}

#[tokio::test]
async fn queued_replica_mutations_never_choose_the_replacement_account() {
    let fixture = NativeAccounts::new();
    let replica = captured(&fixture);
    let item = profile("A");
    let put = replica.put(ReplicaScope::Personal, item.clone(), None);
    let delete = replica.delete(ReplicaScope::Personal, item.id, ReplicaVersion::new(1));
    fixture.login(1);
    assert!(matches!(put.await, Err(ReplicaError::Stale)));
    assert!(matches!(delete.await, Err(ReplicaError::Stale)));
    assert!(
        fixture
            .hub(0)
            .collections(PROFILES_KIND)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        fixture
            .hub(1)
            .collections(PROFILES_KIND)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn late_replica_changes_publish_no_page_for_b() {
    let fixture = NativeAccounts::new();
    let a = captured(&fixture);
    a.put(ReplicaScope::Personal, profile("A"), None)
        .await
        .unwrap();
    let (entered, release) = fixture.hub(0).pause_next(StoreBoundary::ChangesReady);
    let old = a.changes(None);
    entered.wait().await;
    fixture.login(1);
    let b = captured(&fixture);
    assert!(b.changes(None).await.unwrap().changes.is_empty());
    release.release();
    assert!(matches!(old.await, Err(ReplicaError::Stale)));
}

#[tokio::test]
async fn a_finished_replica_page_is_rechecked_when_its_caller_first_polls() {
    let fixture = NativeAccounts::new();
    let replica = captured(&fixture);
    replica
        .put(ReplicaScope::Personal, profile("A"), None)
        .await
        .unwrap();
    let calls = fixture.hub(0).change_calls();
    let old = replica.changes(None);
    while fixture.hub(0).change_calls() == calls {
        tokio::task::yield_now().await;
    }
    fixture.login(1);
    assert!(matches!(old.await, Err(ReplicaError::Stale)));
}

#[tokio::test]
async fn an_old_replica_stays_stale_after_same_account_login_and_a_b_a() {
    let fixture = NativeAccounts::new();
    let old = captured(&fixture);
    fixture.login(0);
    assert!(matches!(old.changes(None).await, Err(ReplicaError::Stale)));
    let next = captured(&fixture);
    fixture.login(1);
    fixture.login(0);
    assert!(matches!(
        next.put(ReplicaScope::Personal, profile("old"), None).await,
        Err(ReplicaError::Stale)
    ));
    assert!(
        captured(&fixture)
            .changes(None)
            .await
            .unwrap()
            .changes
            .is_empty()
    );
}

#[tokio::test]
async fn a_queued_watch_tick_cannot_outlive_its_original_account_capture() {
    let fixture = NativeAccounts::new();
    let replica = captured(&fixture);
    replica
        .put(ReplicaScope::Personal, profile("A"), None)
        .await
        .unwrap();
    let (entered, release) = fixture.hub(0).pause_next(StoreBoundary::WatchReady);
    let mut old = replica.watch();
    entered.wait().await;
    release.release();
    tokio::task::yield_now().await;
    fixture.login(1);
    assert_eq!(old.next().await, None);
}

#[tokio::test]
async fn a_valid_source_from_another_hub_cannot_select_this_accounts_ports() {
    let a = NativeAccounts::new();
    let b = NativeAccounts::at(crate::account::testing::hub_for("other.test"));
    assert!(a.source().check_current().is_ok());
    assert!(b.source().check_current().is_ok());
    assert!(matches!(
        a.operations().bind(b.source()),
        Err(HubError::Stale)
    ));
    captured(&a)
        .put(ReplicaScope::Personal, profile("A"), None)
        .await
        .unwrap();
    assert!(captured(&b).changes(None).await.unwrap().changes.is_empty());
}
