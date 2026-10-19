//! The real managed resolver retains the legacy native filesystem decisions.
use super::*;
use futures::executor::block_on;
use nocterm_ai::{
    connection::{ManagedStartupHandle, ManagedStartupRequest},
    history::source::SourceProducer,
    model::retention::{RetentionAuthority, document::DocumentRetention},
    registry::AgentLaunch,
    workspace::IsolationPaths,
};
fn launch() -> AgentLaunch {
    AgentLaunch {
        id: "test".into(),
        name: "Test".into(),
        command: "test".into(),
        args: vec![],
        env: Default::default(),
        inherit_env: vec![],
    }
}
#[test]
fn managed_workspace_matches_legacy_root_policy_and_native_error_messages() {
    let directory = tempfile::tempdir().unwrap();
    let child = directory.path().join("child-é");
    std::fs::create_dir(&child).unwrap();
    let file = directory.path().join("file");
    std::fs::write(&file, b"x").unwrap();
    #[cfg(unix)]
    let alias = {
        let alias = directory.path().join("alias");
        std::os::unix::fs::symlink(&child, &alias).unwrap();
        alias
    };
    #[cfg(not(unix))]
    let alias = child.clone();
    let resolver = NativeWorkdirResolver::with_home(Some(directory.path().to_owned()));
    for root in [
        child.clone(),
        alias,
        file.clone(),
        file.join("child"),
        directory.path().join("missing"),
        PathBuf::from("relative"),
    ] {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
        let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
        let lease = producer.capture().unwrap();
        let context = lease.context().unwrap();
        let launch = launch();
        let resources = nocterm_ai::AgentResourceSettings::default();
        let isolation = IsolationPaths {
            private_dirs: vec![directory.path().join("private")],
            shared_dirs: vec![],
        };
        let startup = context
            .prepare_startup(ManagedStartupRequest {
                launch: &launch,
                working_directory: &root,
                create_private: false,
                isolation: Some(&isolation),
                terminal_auth: false,
                resources: &resources,
            })
            .unwrap();
        let handle = ManagedStartupHandle::new(&context, startup).unwrap();
        let expected = block_on(resolver.prepare(WorkdirRequest {
            candidate: root.clone(),
            create_private: false,
            isolation: Some(isolation),
        }));
        let actual = block_on(
            resolver
                .prepare_managed(handle, lease.into_lifetime().unwrap())
                .unwrap(),
        );
        match (actual, expected) {
            (Ok(actual), Ok(expected)) => assert_eq!(actual.get(), &expected),
            (Err(actual), Err(expected)) => assert_eq!(actual.to_string(), expected.to_string()),
            (actual, expected) => panic!("managed {},legacy {expected:?}", actual.is_ok()),
        }
        assert!(producer.capture().is_ok());
    }
}
