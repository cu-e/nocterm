use super::*;
use std::error::Error;
#[test]
fn native_causes_remain_concrete_in_every_standard_source_chain() {
    let path = "/etc/os-release".to_owned();
    let errors = [
        OsObservationError::Download {
            path: path.clone(),
            source: FsError::NotConnected,
        },
        OsObservationError::Read {
            path: path.clone(),
            source: FsError::NotConnected,
        },
        OsObservationError::Stat {
            path: path.clone(),
            source: FsError::NotConnected,
        },
        OsObservationError::Close {
            path,
            source: FsError::NotConnected,
        },
        OsObservationError::Home(FsError::NotConnected),
    ];
    for error in errors {
        assert_eq!(
            error.source().unwrap().downcast_ref::<FsError>(),
            Some(&FsError::NotConnected)
        );
    }
    let utf8 = String::from_utf8(vec![0xff]).unwrap_err();
    let error = OsObservationError::InvalidUtf8 {
        path: "/etc/os-release".into(),
        source: utf8,
    };
    assert!(
        error
            .source()
            .unwrap()
            .downcast_ref::<std::string::FromUtf8Error>()
            .is_some()
    );
}
