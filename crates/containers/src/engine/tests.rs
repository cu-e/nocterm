use std::sync::Mutex;

use futures::{FutureExt as _, executor::block_on};
use nocterm_session::{ExecExit, ExecFuture};

use super::*;
use crate::State;

/// What a fake host does with a command line.
type Answer = Result<(&'static str, u32, &'static str), ExecError>;

/// A host that answers each program by the first word pair it starts with.
struct FakeHost {
    answers: Vec<(&'static str, Answer)>,
    ran: Mutex<Vec<String>>,
}

impl FakeHost {
    fn new(answers: Vec<(&'static str, Answer)>) -> Self {
        Self {
            answers,
            ran: Mutex::default(),
        }
    }

    fn ran(&self) -> Vec<String> {
        self.ran.lock().unwrap().clone()
    }
}

impl HostExec for FakeHost {
    fn exec(&self, request: ExecRequest) -> ExecFuture<ExecOutput> {
        let line = request.command_line();
        self.ran.lock().unwrap().push(line.clone());
        let answer = self
            .answers
            .iter()
            .find(|(prefix, _)| line.starts_with(prefix))
            .map(|(_, answer)| answer.clone())
            .unwrap_or(Ok(("", NOT_FOUND, "command not found")));
        let started = answer.map(|(stdout, status, stderr)| {
            let (sink, output) = ExecOutput::channel();
            assert_eq!(
                sink.send(stdout.as_bytes().to_vec()).now_or_never(),
                Some(true)
            );
            sink.finish(ExecExit {
                status: Some(status),
                stderr: stderr.into(),
            });
            output
        });
        Box::pin(async move { started })
    }
}

const CONTAINER: &str =
    r#"{"ID":"a1","Image":"alpine","Names":"box","State":"running","Status":"Up"}"#;
const IMAGE: &str = r#"{"ID":"sha256:1","Repository":"alpine","Tag":"3","Size":"7MB"}"#;

#[test]
fn docker_is_looked_for_first() {
    let host = FakeHost::new(vec![
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
    let host = FakeHost::new(vec![
        ("docker", Err(ExecError::NotFound("docker".into()))),
        ("podman ps", Ok(("[]", 0, ""))),
        ("podman images", Ok(("[]", 0, ""))),
    ]);
    let (engine, snapshot) = block_on(detect(&host)).unwrap();
    assert_eq!(engine, Engine::Podman);
    assert_eq!(snapshot, Snapshot::default());
}

#[test]
fn an_installed_engine_explains_its_failure() {
    let daemon = "Cannot connect to the Docker daemon. Is the docker daemon running?";
    let host = FakeHost::new(vec![("docker", Ok(("", 1, daemon)))]);
    assert_eq!(
        block_on(detect(&host)),
        Err(ContainersError::Failed(daemon.into()))
    );
    let nothing = FakeHost::new(Vec::new());
    assert_eq!(
        block_on(detect(&nothing)),
        Err(ContainersError::NotInstalled)
    );
    let offline = FakeHost::new(vec![("docker", Err(ExecError::Disconnected))]);
    assert_eq!(
        block_on(detect(&offline)),
        Err(ContainersError::Disconnected)
    );
    let ran = offline.ran();
    assert!(ran.iter().all(|line| line.starts_with("docker")), "{ran:?}");
}

#[test]
fn actions_name_every_container() {
    let host = FakeHost::new(vec![
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
    assert_eq!(
        stopped,
        Err(ContainersError::Failed(
            "Error: no container with name or ID \"gone\"".into()
        ))
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
    let host = FakeHost::new(vec![("docker events", Ok(("", 0, "")))]);
    block_on(Engine::Docker.events(&host)).unwrap();
    assert_eq!(
        host.ran(),
        ["docker events --filter type=container --filter type=image"]
    );
}
