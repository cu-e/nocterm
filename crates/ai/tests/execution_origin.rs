//! Captured conversation membership never issues native execution authority.
use nocterm_ai::{
    execution_origin::{ChatKey, ExecutionOrigin, ExecutionOriginError},
    history::ContextId,
};
use nocterm_project_model::ProjectId;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
const ID: &str = "00000000-0000-4000-8000-000000000001";
fn project() -> ContextId {
    ContextId::Project(ProjectId::from_bytes([1; 16]))
}
#[test]
fn equal_chat_ids_in_different_contexts_are_different_document_keys() {
    let general = ChatKey::new(ContextId::General, ID.into());
    let project = ChatKey::new(project(), ID.into());
    assert_ne!(general, project);
    let documents = HashMap::from([(general.clone(), "general"), (project.clone(), "project")]);
    assert_eq!(documents.len(), 2);
    assert_eq!(documents[&general], "general");
    assert_eq!(documents[&project], "project");
    assert_eq!(project.id(), ID);
    assert_eq!(project.scope(), self::project());
}
#[test]
fn an_origin_retains_its_original_context_and_local_provider_resume_candidate() {
    let key = ChatKey::new(project(), ID.into());
    let mut supplied = PathBuf::from("/private/provider/session");
    let origin = ExecutionOrigin::new(key.clone(), Some(supplied.clone()));
    supplied.push("other");
    assert_eq!(origin.key(), &key);
    assert_eq!(origin.scope(), project());
    assert_eq!(
        origin.provider_resume(),
        Some(Path::new("/private/provider/session"))
    );
    let neighbor = ExecutionOrigin::new(
        ChatKey::new(project(), "00000000-0000-4000-8000-000000000002".into()),
        origin.provider_resume().map(Path::to_owned),
    );
    assert_ne!(neighbor.key(), origin.key());
}
#[test]
fn a_consumer_cannot_reassign_an_existing_origin_to_its_own_context() {
    let origin = ExecutionOrigin::new(ChatKey::new(project(), ID.into()), None);
    assert_eq!(
        origin.require_scope(ContextId::General),
        Err(ExecutionOriginError::ScopeMismatch {
            expected: ContextId::General,
            actual: project()
        })
    );
    assert!(origin.require_scope(project()).is_ok());
    assert_eq!(origin.scope(), project());
}
