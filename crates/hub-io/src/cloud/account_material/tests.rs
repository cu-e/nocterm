use super::*;
use nocterm_cloud_model::{
    coordinator::IntakePoolLimits,
    journal::{PublicationAllocation, PublicationBudget, PublicationWorkBudget},
};

#[test]
fn account_material_shared_error_cause_retains_its_actual_work_guard_and_typed_chain() {
    let budget = PublicationBudget::maintenance(IntakePoolLimits::new(1024, 1, 1).unwrap());
    let allocation = budget
        .try_reserve(PublicationAllocation::new(1024, 1).unwrap())
        .unwrap();
    let original = SharedCause::new(std::io::Error::other("foreign metadata"));
    let error = retain_error(AccountMaterialError::Backend(original), allocation);
    let public = SharedCause::new(error);
    let mut current: &(dyn std::error::Error + 'static) = public.as_ref();
    let mut shared = None;
    let mut leaf = false;
    loop {
        if let Some(cause) = current.downcast_ref::<SharedCause>() {
            shared = Some(cause.clone());
        }
        leaf |= current.downcast_ref::<std::io::Error>().is_some();
        match current.source() {
            Some(next) => current = next,
            None => break,
        }
    }
    assert!(leaf);
    let shared = shared.unwrap();
    drop(public);
    assert!(
        budget
            .try_reserve(PublicationAllocation::new(1024, 1).unwrap())
            .is_err()
    );
    drop(shared);
    assert!(
        budget
            .try_reserve(PublicationAllocation::new(1024, 1).unwrap())
            .is_ok()
    );
}
