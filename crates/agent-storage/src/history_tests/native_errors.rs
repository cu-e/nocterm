//! Native publication and deletion failures keep their original concrete causes.
use super::*;
use std::error::Error;

fn concrete<'a, E: Error + 'static>(error: &'a (dyn Error + 'static)) -> Option<&'a E> {
    std::iter::successors(Some(error), |&error| error.source())
        .find_map(|error| error.downcast_ref())
}
fn assert_retained<E: Error + 'static>(error: &AgentStorageError, held: &E) {
    assert!(std::ptr::eq(
        concrete::<E>(error).expect("original cause"),
        held
    ));
    let clone = error.clone();
    assert!(std::ptr::eq(concrete::<E>(&clone).unwrap(), held));
    assert_eq!(clone.to_string(), error.to_string());
}
/// A non-empty directory where the chat transcript belongs.
fn blocked_transcript() -> (tempfile::TempDir, SavedChat, PathBuf) {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let chat = chat(&authority);
    let path = dir.path().join(format!("{}.json", chat.id()));
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("marker"), b"kept").unwrap();
    (dir, chat, path)
}

#[test]
fn a_blocked_transcript_publication_keeps_its_file_error_and_native_cause() {
    let (dir, chat, path) = blocked_transcript();
    let error = save(dir.path(), &chat).unwrap_err();
    let AgentStorageError::Publish {
        path: reported,
        source,
    } = &error
    else {
        panic!("a directory at the transcript path fails publication: {error:?}");
    };
    assert_eq!(reported, &path);
    assert_retained(&error, source.as_ref());
    let native = concrete::<std::io::Error>(&error).expect("native cause below the file error");
    assert!(concrete::<std::io::Error>(source.as_ref()).is_some_and(|io| std::ptr::eq(io, native)));
    assert_eq!(
        error.to_string(),
        format!("Could not publish {}: {}", path.display(), source.as_ref())
    );
    assert_eq!(std::fs::read(path.join("marker")).unwrap(), b"kept");
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(entries, [path], "the private stage is removed");
}

#[test]
fn deleting_a_blocked_transcript_keeps_its_native_delete_error() {
    let (dir, chat, path) = blocked_transcript();
    let error = delete(dir.path(), chat.id()).unwrap_err();
    let AgentStorageError::Delete {
        path: reported,
        source,
    } = &error
    else {
        panic!("a directory at the transcript path cannot be deleted: {error:?}");
    };
    assert_eq!(reported, &path);
    assert_retained(&error, source.as_ref());
    assert_ne!(source.as_ref().kind(), std::io::ErrorKind::NotFound);
    assert_eq!(
        error.to_string(),
        format!("Could not delete {}: {}", path.display(), source.as_ref())
    );
    assert_eq!(std::fs::read(path.join("marker")).unwrap(), b"kept");
}
