use super::*;
use futures::executor::block_on;

#[test]
fn unpolled_and_dropped_launches_do_not_reserve_a_native_start() {
    let opener = LocalProgramOpener::default();
    let request = ProgramOpenRequest::new("not-a-real-program".into(), Vec::new(), None).unwrap();
    drop(opener.open(request));
    let permit = opener.slots.reserve().unwrap();
    drop(permit);
}

#[test]
fn failed_worker_creation_cannot_launch_a_child_and_releases_admission() {
    let slots = Arc::new(LaunchSlots::new(1));
    let permit = slots.reserve().unwrap();
    let (sender, receiver) = oneshot::channel();
    let request = ProgramOpenRequest::new("not-a-real-program".into(), Vec::new(), None).unwrap();
    let error = spawn_worker(request, permit, sender, |_| {
        Err(io::ErrorKind::WouldBlock.into())
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    assert!(block_on(receiver).is_err());
    drop(slots.reserve().unwrap());
}

#[test]
fn unreaped_capacity_is_retired_instead_of_reused() {
    let slots = Arc::new(LaunchSlots::new(1));
    slots.reserve().unwrap().retire();
    assert!(matches!(
        slots.reserve(),
        Err(ProgramOpenError::Busy { limit: 1 })
    ));
}

#[test]
fn missing_program_keeps_native_error_source_and_releases_capacity() {
    let opener = LocalProgramOpener {
        slots: Arc::new(LaunchSlots::new(1)),
    };
    let request =
        ProgramOpenRequest::new("nocterm-nonexistent-opener-bc6fac".into(), Vec::new(), None)
            .unwrap();
    let error = block_on(opener.open(request)).unwrap_err();
    assert!(
        matches!(error, ProgramOpenError::Start { ref program, ref source } if program == "nocterm-nonexistent-opener-bc6fac" && source.kind() == io::ErrorKind::NotFound)
    );
    // A failed start is settled only after its worker releases admission.
    let request =
        ProgramOpenRequest::new("nocterm-nonexistent-opener-bc6fac".into(), Vec::new(), None)
            .unwrap();
    assert!(matches!(
        block_on(opener.open(request)),
        Err(ProgramOpenError::Start { .. })
    ));
}

#[cfg(unix)]
mod native;
