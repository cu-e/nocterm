//! Borrowed launch preparation owns each actual copy before native startup.
use super::*;
use crate::{
    history::source::SourceProducer,
    model::{
        native::NativeBytes,
        retention::{RetentionAuthority, document::DocumentRetention},
    },
};
fn launch() -> AgentLaunch {
    AgentLaunch {
        id: "original-agent".into(),
        name: "Original agent".into(),
        command: "native-agent".into(),
        args: vec!["--原".into(), "two words".into()],
        env: [
            ("Z_NAME".into(), "last".into()),
            ("A_NAME".into(), "first".into()),
        ]
        .into_iter()
        .collect(),
        inherit_env: vec!["ORIGINAL_AUTH_ENV".into()],
    }
}
#[test]
fn copied_native_paths_preserve_original_lexical_bytes_and_refuse_closed_source() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
    let lease = producer.capture().unwrap();
    let context = lease.context().unwrap();
    let baseline = context.ledger().live_requested_bytes();
    let path = Path::new("./original/../workspace//leaf");
    let native = context.copy_path(path).unwrap();
    assert_eq!(native.as_path().as_os_str(), path.as_os_str());
    assert_eq!(
        context.ledger().live_requested_bytes(),
        baseline + path.as_os_str().len()
    );
    drop(lease);
    assert!(matches!(
        context.copy_path(path),
        Err(JsonDecodeError::Retention(
            crate::model::retention::RetentionError::FrozenOwner
        ))
    ));
    assert_eq!(native.as_path().as_os_str(), path.as_os_str());
    drop(native);
    assert_eq!(context.ledger().live_requested_bytes(), baseline);
}
#[test]
fn startup_copies_preserve_original_fields_and_native_path_until_real_owner_drop() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
    let lease = producer.capture().unwrap();
    let context = lease.context().unwrap();
    let original = launch();
    let root = Path::new("./original/../workspace//");
    let isolation = IsolationPaths {
        private_dirs: vec![PathBuf::from("private/../native")],
        shared_dirs: vec![PathBuf::from("shared//socket")],
    };
    let resources = AgentResourceSettings::default();
    let baseline = context.ledger().live_requested_bytes();
    let facts = context
        .prepare_startup(ManagedStartupRequest {
            launch: &original,
            working_directory: root,
            create_private: true,
            isolation: Some(&isolation),
            terminal_auth: true,
            resources: &resources,
        })
        .unwrap();
    assert!(context.ledger().live_requested_bytes() > baseline);
    drop(lease);
    let actual = facts.get();
    assert_eq!(actual.launch().id(), original.id);
    assert_eq!(actual.launch().name(), original.name);
    assert_eq!(actual.launch().command(), original.command);
    assert_eq!(actual.launch().args(), original.args);
    assert_eq!(actual.launch().inherit_env(), original.inherit_env);
    assert_eq!(
        actual
            .launch()
            .env()
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect::<Vec<_>>(),
        original
            .env
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect::<Vec<_>>()
    );
    assert_eq!(actual.working_directory(), root);
    assert_eq!(
        actual.isolation().unwrap().private_dirs(),
        isolation.private_dirs
    );
    assert_eq!(
        actual.isolation().unwrap().shared_dirs(),
        isolation.shared_dirs
    );
    assert_eq!(actual.resources(), &resources);
    assert!(actual.create_private() && actual.terminal_auth());
    drop(facts);
    assert_eq!(context.ledger().live_requested_bytes(), baseline);
}
#[test]
fn refused_startup_keeps_existing_launch_owner_and_releases_only_unpublished_copies() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
    let lease = producer.capture().unwrap();
    let context = lease.context().unwrap();
    let mut original = launch();
    let resources = AgentResourceSettings::default();
    let kept = context
        .prepare_startup(request(&original, &resources))
        .unwrap();
    let pointer = kept.get().launch().name().as_ptr();
    let source_buffer = NativeBytes::new(context.ledger(), 56 * 1024 * 1024).unwrap();
    original.name = "n".repeat(8 * 1024 * 1024);
    let before = context.ledger().live_requested_bytes();
    assert!(matches!(
        context.prepare_startup(request(&original, &resources)),
        Err(JsonDecodeError::Limit { .. })
    ));
    assert_eq!(context.ledger().live_requested_bytes(), before);
    assert_eq!(kept.get().launch().name(), "Original agent");
    assert_eq!(kept.get().launch().name().as_ptr(), pointer);
    assert_eq!(original.name.len(), 8 * 1024 * 1024);
    drop(source_buffer);
}

fn request<'a>(
    launch: &'a AgentLaunch,
    resources: &'a AgentResourceSettings,
) -> ManagedStartupRequest<'a> {
    ManagedStartupRequest {
        launch,
        working_directory: Path::new("original"),
        create_private: false,
        isolation: None,
        terminal_auth: false,
        resources,
    }
}
