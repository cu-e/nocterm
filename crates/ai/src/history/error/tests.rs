use super::HistoryCodecError;
use crate::storage::AgentStorageError;
use std::error::Error;

fn concrete<'a, E: Error + 'static>(error: &'a (dyn Error + 'static)) -> Option<&'a E> {
    std::iter::successors(Some(error), |&error| error.source())
        .find_map(|error| error.downcast_ref())
}

#[test]
fn a_json_failure_keeps_its_original_parser_error_in_the_source_chain() {
    let parser = serde_json::from_str::<serde_json::Value>("{\"id\": ").unwrap_err();
    let (line, column, category) = (parser.line(), parser.column(), parser.classify());
    let message = format!("Could not encode or decode the saved chat: {parser}");
    let error = HistoryCodecError::from(parser);
    let HistoryCodecError::Json(held) = &error else {
        panic!("JSON failures stay Json: {error:?}");
    };
    let found = concrete::<serde_json::Error>(&error).expect("original parser error");
    assert!(std::ptr::eq(found, held.as_ref()));
    assert_eq!(
        (found.line(), found.column(), found.classify()),
        (line, column, category)
    );
    assert_eq!(error.to_string(), message);

    let clone = error.clone();
    let cloned = concrete::<serde_json::Error>(&clone).expect("clone keeps the parser error");
    assert!(std::ptr::eq(cloned, found));
    assert_eq!(clone.to_string(), message);

    let original: *const serde_json::Error = found;
    let storage = AgentStorageError::from(error);
    let through = concrete::<serde_json::Error>(&storage).expect("codec wrapper keeps it");
    assert!(std::ptr::eq(through, original));
    assert_eq!(storage.to_string(), message);
}
