//! Provider access that arrives without a command resumes the parked prompt.
use super::*;

async fn parked_for_provider_access() -> (Fixture, RunBinding) {
    let fixture = Fixture::new();
    fixture.providers.revoke();
    fixture
        .actor
        .accept(submit(1, ChatControlRevision::default()))
        .await
        .unwrap();
    let run = fixture.binding().await;
    fixture
        .actor
        .accept(inference(2, fixture.control().await, run.clone()))
        .await
        .unwrap();
    let _ = fixture.actor.drive_one().await;
    assert_eq!(
        fixture.actor.status(&run).await.unwrap(),
        RunStatus::NeedsProviderAccess
    );
    (fixture, run)
}

#[tokio::test]
async fn a_provider_wake_resumes_the_prompt_parked_for_provider_access() {
    let (fixture, run) = parked_for_provider_access().await;
    assert!(
        !fixture.actor.drive_one().await.unwrap(),
        "nothing wakes it"
    );
    fixture.providers.restore();
    fixture.engine.push(complete());
    fixture
        .actor
        .wake_provider(stream().chat(), &NamespaceId::new("provider").unwrap());
    assert!(fixture.actor.drive_one().await.unwrap());
    assert_eq!(
        fixture.actor.status(&run).await.unwrap(),
        RunStatus::Completed
    );
}

#[tokio::test]
async fn a_wake_for_another_chat_or_provider_leaves_the_prompt_parked() {
    let (fixture, run) = parked_for_provider_access().await;
    fixture.providers.restore();
    fixture.engine.push(complete());
    fixture.actor.wake_provider(
        CloudChatId::from_bytes([9; 16]),
        &NamespaceId::new("provider").unwrap(),
    );
    fixture
        .actor
        .wake_provider(stream().chat(), &NamespaceId::new("elsewhere").unwrap());
    assert!(!fixture.actor.drive_one().await.unwrap());
    assert_eq!(
        fixture.actor.status(&run).await.unwrap(),
        RunStatus::NeedsProviderAccess
    );
}
