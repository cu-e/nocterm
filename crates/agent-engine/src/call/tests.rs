use super::*;
use nocterm_cloud_model::testing::{ScriptedEngineEvents, ScriptedModelAdmission};

fn context(step: u8, admission: Arc<ScriptedModelAdmission>) -> AdvanceContext {
    let (cancellation, _) = AbortHandle::new_pair();
    AdvanceContext::new(
        crate::tests::config().run,
        StepId::from_bytes([step; 16]),
        Arc::new(ScriptedEngineEvents::default()),
        admission,
        cancellation,
    )
}

#[tokio::test]
async fn one_scope_cannot_admit_a_second_http_request() {
    let run = crate::tests::config().run;
    let calls = ModelCalls::new(run.clone());
    let admission = Arc::new(ScriptedModelAdmission::new(run));
    let lease = calls.begin(&context(1, admission.clone()), 1024).unwrap();
    let scope = calls.scope().unwrap();
    assert!(scope.reserve(80).await.is_ok());
    assert!(matches!(
        scope.reserve(80).await,
        Err(EngineError::InvalidStream)
    ));
    assert_eq!(admission.requests().len(), 1);
    assert!(matches!(lease.failure(), Some(EngineError::InvalidStream)));
}

#[tokio::test]
async fn a_retired_scope_cannot_retarget_a_late_ack_to_its_replacement() {
    let run = crate::tests::config().run;
    let calls = ModelCalls::new(run.clone());
    let admission = Arc::new(ScriptedModelAdmission::new(run));
    let (entered, release) = admission.pause_next();
    let lease = calls.begin(&context(1, admission.clone()), 1024).unwrap();
    let old = calls.scope().unwrap();
    let pending = old.reserve(80);
    tokio::pin!(pending);
    tokio::select! { result=&mut pending=>panic!("reservation must wait: {result:?}"), _=entered.wait()=>{} }
    drop(lease);
    let replacement = calls.begin(&context(2, admission.clone()), 512).unwrap();
    release.release();
    assert!(matches!(pending.await, Err(EngineError::Interrupted)));
    let request = calls.scope().unwrap().reserve(120).await.unwrap();
    assert_eq!(request.step(), StepId::from_bytes([2; 16]));
    assert_eq!(request.output_token_limit(), 512);
    assert!(replacement.failure().is_none());
}

#[tokio::test]
async fn a_cancelled_scope_refuses_admission_before_host_reservation() {
    let run = crate::tests::config().run;
    let calls = ModelCalls::new(run.clone());
    let admission = Arc::new(ScriptedModelAdmission::new(run));
    let (cancellation, _) = AbortHandle::new_pair();
    let context = AdvanceContext::new(
        crate::tests::config().run,
        StepId::from_bytes([1; 16]),
        Arc::new(ScriptedEngineEvents::default()),
        admission.clone(),
        cancellation.clone(),
    );
    cancellation.abort();
    let _lease = calls.begin(&context, 1024).unwrap();
    assert!(matches!(
        calls.scope().unwrap().reserve(80).await,
        Err(EngineError::Interrupted)
    ));
    assert!(admission.requests().is_empty());
}

#[tokio::test]
async fn observed_attempts_are_idempotent_but_changed_observation_conflicts() {
    let run = crate::tests::config().run;
    let calls = ModelCalls::new(run.clone());
    let admission = Arc::new(ScriptedModelAdmission::new(run));
    let lease = calls.begin(&context(1, admission.clone()), 1024).unwrap();
    calls.scope().unwrap().reserve(80).await.unwrap();
    let observed = ModelCallObservation::new(Default::default(), 10).unwrap();
    lease.reconcile(observed.clone()).await.unwrap();
    lease.reconcile(observed).await.unwrap();
    assert_eq!(admission.observations().len(), 1);
    assert!(matches!(
        lease
            .reconcile(ModelCallObservation::new(Default::default(), 11).unwrap())
            .await,
        Err(EngineError::InvalidStream)
    ));
    assert_eq!(admission.observations().len(), 1);
}
