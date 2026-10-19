use futures::executor::block_on;
use nocterm_session::{ExecError, testing::FakeExec};

use super::*;
use crate::State;

const CONTAINER: &str =
    r#"{"ID":"a1","Image":"alpine","Names":"box","State":"running","Status":"Up"}"#;
const IMAGE: &str = r#"{"ID":"sha256:1","Repository":"alpine","Tag":"3","Size":"7MB"}"#;

#[test]
fn docker_is_looked_for_first() {
    let host = FakeExec::new(vec![
        ("docker ps --all", Ok((CONTAINER, 0, ""))),
        ("docker images --all", Ok((IMAGE, 0, ""))),
    ]);
    let (engine, snapshot) = block_on(detect(&host)).unwrap();
    assert_eq!(engine, Engine::Docker);
    assert_eq!(snapshot.containers[0].state, State::Running);
    assert_eq!(snapshot.images[0].name(), "alpine:3");
    assert!(
        host.ran()
            .iter()
            .all(|line| line.ends_with("--format json"))
    );
}

#[test]
fn podman_answers_where_docker_is_missing() {
    let host = FakeExec::new(vec![
        ("docker", Err(ExecError::NotFound("docker".into()))),
        ("podman ps", Ok(("[]", 0, ""))),
        ("podman images", Ok(("[]", 0, ""))),
    ]);
    let (engine, snapshot) = block_on(detect(&host)).unwrap();
    assert_eq!(engine, Engine::Podman);
    assert_eq!(snapshot, Snapshot::default());
}

#[test]
fn exit_127_is_engine_absence_even_with_stderr() {
    let host = FakeExec::new(vec![("docker", Ok(("", 127, "command not found")))]);
    assert!(matches!(
        block_on(Engine::Docker.snapshot(&host)),
        Err(ContainersError::NotInstalled)
    ));
}

#[test]
fn an_installed_engine_explains_its_failure() {
    let daemon = "Cannot connect to the Docker daemon. Is the docker daemon running?";
    let host = FakeExec::new(vec![("docker", Ok(("", 1, daemon)))]);
    let error = block_on(detect(&host)).unwrap_err();
    assert!(
        matches!(&error, ContainersError::Exit { status: Some(1), stderr_tail } if stderr_tail == daemon)
    );
    assert_eq!(error.to_string(), daemon);
    let nothing = FakeExec::new(Vec::new());
    assert!(matches!(
        block_on(detect(&nothing)),
        Err(ContainersError::NotInstalled)
    ));
    let offline = FakeExec::new(vec![("docker", Err(ExecError::Disconnected))]);
    assert!(matches!(
        block_on(detect(&offline)),
        Err(ContainersError::Disconnected)
    ));
    let ran = offline.ran();
    assert!(ran.iter().all(|line| line.starts_with("docker")), "{ran:?}");
}

#[test]
fn actions_name_every_container() {
    let host = FakeExec::new(vec![
        ("podman rm", Ok(("", 0, ""))),
        (
            "podman stop",
            Ok(("", 1, "Error: no container with name or ID \"gone\"")),
        ),
    ]);
    let ids = ["a1".to_owned(), "b2".to_owned()];
    block_on(Engine::Podman.perform(&host, Action::Remove, &ids)).unwrap();
    assert_eq!(host.ran(), ["podman rm --force a1 b2"]);
    let stopped = block_on(Engine::Podman.perform(&host, Action::Stop, &ids[..1]));
    let error = stopped.unwrap_err();
    assert!(matches!(
        &error,
        ContainersError::Exit {
            status: Some(1),
            ..
        }
    ));
    assert_eq!(
        error.to_string(),
        "Error: no container with name or ID \"gone\""
    );
}

#[test]
fn logs_and_shells_are_programs_for_a_terminal() {
    assert_eq!(
        Engine::Docker.logs("a1").command_line(),
        "docker logs --follow --tail 1000 a1"
    );
    assert_eq!(
        Engine::Podman.shell("a1").command_line(),
        "podman exec --interactive --tty a1 sh -c \
         'command -v bash >/dev/null 2>&1 && exec bash || exec sh'"
    );
    let host = FakeExec::new(vec![("docker events", Ok(("", 0, "")))]);
    block_on(Engine::Docker.events(&host)).unwrap();
    assert_eq!(
        host.ran(),
        ["docker events --filter type=container --filter type=image"]
    );
}

#[test]
fn successful_invalid_output_preserves_its_native_json_source() {
    let host = FakeExec::new(vec![
        ("docker ps", Ok(("\n{}\n\nnot JSON", 0, ""))),
        ("docker images", Ok((IMAGE, 0, ""))),
    ]);
    let error = block_on(detect(&host)).unwrap_err();
    let ContainersError::Parse(crate::ListingError::Json { line, source }) = &error else {
        panic!("malformed success was mislabeled as engine absence")
    };
    assert_eq!(*line, 4);
    assert!(source.is_syntax());
    assert!(
        std::error::Error::source(&error)
            .unwrap()
            .is::<crate::ListingError>()
    );
    assert!(
        std::error::Error::source(std::error::Error::source(&error).unwrap())
            .unwrap()
            .is::<serde_json::Error>()
    );
    assert!(!error.to_string().contains("not JSON"));
    assert!(host.ran().iter().all(|line| !line.starts_with("sh ")));
}

#[test]
fn listing_objects_and_required_ids_have_distinct_typed_failures() {
    for (output, expected_index, missing) in [
        ("[{\"Id\":\"ok\"},null]", 1, false),
        ("[{\"Id\":\"ok\"},{\"Names\":[\"missing\"]}]", 1, true),
    ] {
        let host = FakeExec::new(vec![
            ("podman ps", Ok((output, 0, ""))),
            ("podman images", Ok(("[]", 0, ""))),
        ]);
        let error = block_on(Engine::Podman.snapshot(&host)).unwrap_err();
        match error {
            ContainersError::Parse(crate::ListingError::ExpectedObject { index }) => {
                assert!(!missing);
                assert_eq!(index, expected_index);
            }
            ContainersError::Parse(crate::ListingError::MissingId { entity, index }) => {
                assert!(missing);
                assert_eq!(entity, crate::ListingEntity::Container);
                assert_eq!(index, expected_index);
            }
            error => panic!("unexpected error: {error:?}"),
        }
    }
    let host = FakeExec::new(vec![
        ("docker ps", Ok((CONTAINER, 0, ""))),
        ("docker images", Ok(("{}", 0, ""))),
    ]);
    assert!(matches!(
        block_on(Engine::Docker.snapshot(&host)),
        Err(ContainersError::Parse(crate::ListingError::MissingId {
            entity: crate::ListingEntity::Image,
            index: 0,
        }))
    ));
}

#[test]
fn startup_failures_keep_exec_sources_and_unsupported_never_falls_back() {
    let host = FakeExec::new(vec![(
        "docker",
        Err(ExecError::Failed("spawn refused".into())),
    )]);
    let error = block_on(detect(&host)).unwrap_err();
    assert!(
        matches!(&error, ContainersError::Execution(ExecError::Failed(message)) if message == "spawn refused")
    );
    assert!(std::error::Error::source(&error).unwrap().is::<ExecError>());
    let host = FakeExec::new(vec![("docker", Err(ExecError::Unsupported))]);
    assert!(matches!(
        block_on(detect(&host)),
        Err(ContainersError::Unsupported)
    ));
    assert!(host.ran().iter().all(|line| line.starts_with("docker")));
}

#[test]
fn failed_command_keeps_status_and_a_utf8_bounded_stderr_tail() {
    let diagnostic = format!("{}\nlatest failure", "é".repeat(5000));
    let host = FakeExec::new(vec![("docker stop", Ok(("", 17, &diagnostic)))]);
    let error = block_on(Engine::Docker.perform(&host, Action::Stop, &["a1".into()])).unwrap_err();
    let ContainersError::Exit {
        status,
        stderr_tail,
    } = &error
    else {
        panic!("expected command exit status")
    };
    assert_eq!(*status, Some(17));
    assert!(stderr_tail.len() <= 4 * 1024);
    assert!(stderr_tail.ends_with("latest failure"));
    assert_eq!(error.to_string(), "latest failure");
}

#[test]
fn malformed_podman_array_keeps_the_native_location_and_eof_cause() {
    let host = FakeExec::new(vec![
        ("podman ps", Ok(("[\n{\"Id\":", 0, ""))),
        ("podman images", Ok(("[]", 0, ""))),
    ]);
    let error = block_on(Engine::Podman.snapshot(&host)).unwrap_err();
    let ContainersError::Parse(crate::ListingError::Json { line, source }) = error else {
        panic!("expected native Podman JSON failure")
    };
    assert_eq!(line, 2);
    assert_eq!(source.line(), 2);
    assert!(source.column() > 0);
    assert!(source.is_eof());
}
