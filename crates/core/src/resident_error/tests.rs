//! Diagnosis and last-drop behavior through the shared owner's public methods.
use super::*;
use std::{
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Barrier, Mutex},
};

#[test]
fn clone_and_standard_sources_preserve_the_original_nonclone_native_leaf() {
    struct Credit;
    let error = SharedResidentError::new(io::Error::from_raw_os_error(13), Credit);
    let clone = error.clone();
    assert!(std::ptr::eq(error.as_ref(), clone.as_ref()));
    assert_eq!(clone.as_ref().raw_os_error(), Some(13));
    let original = clone.source().unwrap().downcast_ref::<io::Error>().unwrap();
    assert!(std::ptr::eq(original, error.as_ref()));
    assert!(original.source().is_none());
    assert_eq!(clone.to_string(), original.to_string());
    assert_eq!(format!("{clone:?}"), format!("{original:?}"));
    drop(error);
    assert_eq!(clone.as_ref().raw_os_error(), Some(13));
}
#[test]
fn original_nonleaf_error_precedes_its_existing_concrete_source() {
    let original = crate::PersistError::Io {
        path: "original.toml".into(),
        source: io::Error::from(io::ErrorKind::PermissionDenied),
    };
    let error = SharedResidentError::new(original, ());
    let wrapper = error
        .source()
        .unwrap()
        .downcast_ref::<crate::PersistError>()
        .unwrap();
    assert!(std::ptr::eq(wrapper, error.as_ref()));
    let native = wrapper
        .source()
        .unwrap()
        .downcast_ref::<io::Error>()
        .unwrap();
    assert_eq!(native.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(error.to_string(), wrapper.to_string());
}

type Events = Arc<Mutex<Vec<&'static str>>>;
struct Payload(Events);
impl Drop for Payload {
    fn drop(&mut self) {
        self.0.lock().unwrap().push("payload");
    }
}
struct Credit(Events);
impl Drop for Credit {
    fn drop(&mut self) {
        self.0.lock().unwrap().push("credit");
    }
}
#[derive(thiserror::Error)]
#[error("owned diagnosis")]
struct OwnedError {
    _payload: Payload,
    events: Events,
    panic: bool,
}
impl fmt::Debug for OwnedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OwnedError")
    }
}
impl Drop for OwnedError {
    fn drop(&mut self) {
        self.events.lock().unwrap().push("error");
        if self.panic {
            panic!("scripted destructor failure");
        }
    }
}
fn owned(events: &Events, panic: bool) -> SharedResidentError<OwnedError, Credit> {
    SharedResidentError::new(
        OwnedError {
            _payload: Payload(events.clone()),
            events: events.clone(),
            panic,
        },
        Credit(events.clone()),
    )
}
#[test]
fn only_last_clone_drops_original_payload_before_returning_credit() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let error = owned(&events, false);
    let last = error.clone();
    drop(error);
    assert!(events.lock().unwrap().is_empty());
    drop(last);
    assert_eq!(*events.lock().unwrap(), ["error", "payload", "credit"]);
}
#[test]
fn destructor_panic_still_drops_error_fields_before_returning_credit() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let error = owned(&events, true);
    let result = catch_unwind(AssertUnwindSafe(|| drop(error)));
    assert!(result.is_err());
    assert_eq!(*events.lock().unwrap(), ["error", "payload", "credit"]);
}
#[test]
fn concurrent_last_clones_drop_one_payload_and_one_credit() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let error = owned(&events, false);
    let barrier = Arc::new(Barrier::new(3));
    let handles = [error.clone(), error].map(|error| {
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            drop(error);
        })
    });
    barrier.wait();
    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(*events.lock().unwrap(), ["error", "payload", "credit"]);
}
