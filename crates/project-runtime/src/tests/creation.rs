use super::*;
use futures::FutureExt as _;
use nocterm_session::{ExecExit, ExecRequest};

fn create_input(engine: Engine) -> ProjectSetupRequest {
    request(SetupMode::Create {
        engine,
        image: WorkspaceImage::Ubuntu,
        languages: vec![],
        root: None,
    })
}
fn attempt(exec: &FakeExec) -> String {
    exec.requests()
        .iter()
        .find(|request| request.args.first().is_some_and(|arg| arg == "create"))
        .unwrap()
        .args
        .iter()
        .find_map(|arg| arg.strip_prefix("io.nocterm.attempt=").map(str::to_owned))
        .unwrap()
}
fn inspection_with_attempt(token: &str) -> String {
    let mut value: serde_json::Value = serde_json::from_str(&inspect("created", true)).unwrap();
    value[0]["Config"]["Labels"]["io.nocterm.attempt"] = token.into();
    value.to_string()
}
fn removal_requests(exec: &FakeExec) -> Vec<ExecRequest> {
    exec.requests()
        .into_iter()
        .filter(|request| request.args.first().is_some_and(|arg| arg == "rm"))
        .collect()
}

#[test]
fn canceled_create_reconciles_daemon_owned_container_by_exact_attempt_and_engine() {
    for engine in Engine::ALL {
        let exec = FakeExec::default();
        let program = engine.name().to_lowercase();
        answer(&exec, &format!("{program} pull"), String::new());
        let sink = exec.queue_stream(format!("{program} create"));
        let cancel = CancelFlag::default();
        let mut pending = NativeProjectPreparation.prepare(
            Arc::new(exec.clone()),
            create_input(engine),
            cancel.clone(),
            Arc::new(|_| {}),
        );
        assert!(pending.as_mut().now_or_never().is_none());
        let token = attempt(&exec);
        answer(&exec, &format!("{program} ps"), id());
        answer(
            &exec,
            &format!("{program} inspect"),
            inspection_with_attempt(&token),
        );
        answer(&exec, &format!("{program} rm"), String::new());
        cancel.cancel();
        assert!(matches!(
            block_on(pending),
            Err(ProjectSetupError::Canceled)
        ));
        assert!(sink.is_closed());
        assert_eq!(
            removal_requests(&exec),
            vec![engine.command().args(["rm", "--force", &id()])]
        );
        assert!(
            exec.requests()
                .iter()
                .all(|request| request.program == program
                    || request.program == "sh"
                        && request.args.get(2).is_some_and(|arg| arg == "nocterm-pull"))
        );
        assert!(
            exec.ran()
                .iter()
                .any(|line| line.contains(&format!("label=io.nocterm.attempt={token}")))
        );
    }
}

#[test]
fn lost_or_malformed_create_output_recovers_only_the_attempt_owned_container() {
    for malformed in [false, true] {
        let exec = FakeExec::default();
        answer(&exec, "docker pull", String::new());
        let sink = exec.queue_stream("docker create");
        let mut pending = NativeProjectPreparation.prepare(
            Arc::new(exec.clone()),
            create_input(Engine::Docker),
            CancelFlag::default(),
            Arc::new(|_| {}),
        );
        assert!(pending.as_mut().now_or_never().is_none());
        answer(&exec, "docker ps", id());
        answer(
            &exec,
            "docker inspect",
            inspection_with_attempt(&attempt(&exec)),
        );
        answer(&exec, "docker rm", String::new());
        if malformed {
            assert!(block_on(sink.send(b"not-a-container-id".to_vec())));
        }
        sink.finish(ExecExit {
            status: malformed.then_some(0),
            stderr: String::new(),
        });
        drop(sink);
        assert!(matches!(
            block_on(pending),
            Err(ProjectSetupError::InvalidReference | ProjectSetupError::CommandFailed { .. })
        ));
        assert_eq!(removal_requests(&exec).len(), 1);
    }
}

#[test]
fn uncertain_create_never_deletes_missing_ambiguous_malformed_or_other_attempt_matches() {
    for listed in [
        String::new(),
        format!("{}\n{}", id(), "b".repeat(64)),
        "short-id".into(),
        id(),
    ] {
        let exec = FakeExec::default();
        answer(&exec, "docker pull", String::new());
        let sink = exec.queue_stream("docker create");
        let mut pending = NativeProjectPreparation.prepare(
            Arc::new(exec.clone()),
            create_input(Engine::Docker),
            CancelFlag::default(),
            Arc::new(|_| {}),
        );
        assert!(pending.as_mut().now_or_never().is_none());
        let token = attempt(&exec);
        answer(&exec, "docker ps", listed);
        answer(
            &exec,
            "docker inspect",
            inspection_with_attempt("other-attempt"),
        );
        sink.finish(ExecExit::default());
        drop(sink);
        let error = block_on(pending).unwrap_err();
        assert!(
            matches!(error, ProjectSetupError::CreationUncertain { engine:Engine::Docker, attempt, .. } if attempt.as_str() == token)
        );
        assert!(removal_requests(&exec).is_empty());
    }
}

#[test]
fn failed_later_setup_cleanup_requires_the_successful_create_receipt_attempt() {
    let exec = FakeExec::default();
    answer(&exec, "docker create", id());
    answer(&exec, "docker pull", String::new());
    answer(&exec, "docker start", String::new());
    answer(&exec, "docker rm", String::new());
    answer(&exec, "docker exec", String::new());
    let install = exec.queue_stream(format!("docker exec --interactive {} sh -s", id()));
    let mut pending = NativeProjectPreparation.prepare(
        Arc::new(exec.clone()),
        request(SetupMode::Create {
            engine: Engine::Docker,
            image: WorkspaceImage::Ubuntu,
            languages: vec![LanguageSelection::default_for(ProgrammingLanguage::Python)],
            root: None,
        }),
        CancelFlag::default(),
        Arc::new(|_| {}),
    );
    assert!(pending.as_mut().now_or_never().is_none());
    answer(
        &exec,
        "docker inspect",
        inspection_with_attempt(&attempt(&exec)),
    );
    install.finish(ExecExit {
        status: Some(1),
        stderr: String::new(),
    });
    drop(install);
    assert!(matches!(
        block_on(pending),
        Err(ProjectSetupError::Environment(_))
    ));
    assert_eq!(removal_requests(&exec).len(), 1);
}

#[test]
fn retries_use_distinct_creation_tokens_even_for_the_same_project() {
    let exec = FakeExec::default();
    answer(&exec, "docker create", id());
    answer(&exec, "docker", String::new());
    let first = prepare(exec.clone(), create_input(Engine::Docker))
        .unwrap()
        .owned
        .unwrap();
    let second = prepare(exec, create_input(Engine::Docker))
        .unwrap()
        .owned
        .unwrap();
    assert_eq!(first.project(), second.project());
    assert_ne!(first.attempt(), second.attempt());
}

#[test]
fn saved_ownership_receipt_cannot_delete_another_attempt_of_the_same_project() {
    let exec = FakeExec::default();
    answer(
        &exec,
        "docker inspect",
        inspection_with_attempt(CreationToken::from_bytes([3; 16]).as_str()),
    );
    let receipt = OwnedContainer::new(
        ContainerLocation::new(
            HostLocator::Local,
            Engine::Docker,
            id(),
            RootPath::parse("/").unwrap(),
            ContainerOrigin::Created,
        )
        .unwrap(),
        ProjectId::from_bytes([1; 16]),
        CreationToken::from_bytes([2; 16]),
    );
    assert!(matches!(
        block_on(NativeProjectPreparation.discard_owned(Arc::new(exec.clone()), receipt)),
        Err(ProjectSetupError::CleanupRefused)
    ));
    assert!(removal_requests(&exec).is_empty());
}

#[test]
fn custom_local_ids_tags_and_registry_references_only_pull_if_missing_on_original_engine() {
    for engine in Engine::ALL {
        for image in [
            format!("sha256:{}", "b".repeat(64)),
            "local/noct-workspace:latest".into(),
            "registry.example/team/image:stable".into(),
        ] {
            let exec = FakeExec::default();
            let program = engine.name().to_lowercase();
            answer(&exec, &format!("{program} create"), id());
            answer(&exec, &format!("{program} start"), String::new());
            answer(&exec, &format!("{program} exec"), String::new());
            let input = request(SetupMode::Create {
                engine,
                image: WorkspaceImage::Custom(image.clone()),
                languages: vec![],
                root: None,
            });
            let prepared = prepare(exec.clone(), input).unwrap();
            assert_eq!(prepared.owned.unwrap().location().engine(), engine);
            let requests = exec.requests();
            assert!(requests.iter().all(|request| request.program == program));
            assert!(requests.iter().all(|request| request.args[0] != "pull"));
            assert!(requests.iter().all(|request| request.stdin.is_none()));
            let create = requests
                .iter()
                .find(|request| request.args[0] == "create")
                .unwrap();
            assert!(create.args.iter().any(|arg| arg == "--pull=missing"));
            assert_eq!(&create.args[create.args.len() - 3], &image);
            assert!(
                create
                    .args
                    .iter()
                    .any(|arg| arg.starts_with("io.nocterm.attempt="))
            );
        }
    }
}
#[test]
fn curated_images_still_pull_before_creating_on_the_selected_engine() {
    for engine in Engine::ALL {
        for image in [WorkspaceImage::Ubuntu, WorkspaceImage::Debian] {
            let exec = FakeExec::default();
            let program = engine.name().to_lowercase();
            answer(&exec, &format!("{program} create"), id());
            answer(&exec, &program, String::new());
            prepare(
                exec.clone(),
                request(SetupMode::Create {
                    engine,
                    image: image.clone(),
                    languages: vec![],
                    root: None,
                }),
            )
            .unwrap();
            let requests = exec.requests();
            assert_eq!(
                requests[0],
                crate::prepare::pull_request(engine, image.reference())
            );
            let create = requests
                .iter()
                .find(|request| request.args[0] == "create")
                .unwrap();
            assert!(!create.args.iter().any(|arg| arg.starts_with("--pull")));
        }
    }
}

#[test]
fn image_pull_progress_streams_before_exit_and_cancellation_stops_the_original_host_command() {
    let exec = FakeExec::default();
    let sink = exec.queue_stream("sh -c");
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = observed.clone();
    let cancel = CancelFlag::default();
    let mut pending = NativeProjectPreparation.prepare(
        Arc::new(exec.clone()),
        create_input(Engine::Docker),
        cancel.clone(),
        Arc::new(move |update| recorded.lock().unwrap().push(update)),
    );
    assert!(pending.as_mut().now_or_never().is_none());
    block_on(sink.send(b"\x1b[2Klayer: Downloading [=>] 1.5MB/3MB\r".to_vec()));
    assert!(pending.as_mut().now_or_never().is_none());
    assert!(observed.lock().unwrap().iter().any(|update| {
        update.transfer.as_ref().is_some_and(|transfer| {
            transfer.current_bytes == 1_500_000 && transfer.total_bytes == Some(3_000_000)
        })
    }));
    cancel.cancel();
    assert!(matches!(
        block_on(pending),
        Err(ProjectSetupError::Canceled)
    ));
    assert!(sink.is_closed());
    assert_eq!(
        exec.requests(),
        vec![crate::prepare::pull_request(
            Engine::Docker,
            WorkspaceImage::Ubuntu.reference()
        )]
    );
}
#[test]
fn a_failed_pull_never_emits_a_creation_phase_or_completes_a_workspace() {
    let exec = FakeExec::default();
    exec.respond(
        "sh -c",
        Ok(ExecResponse::text(
            "layer: Downloading 1MB/2MB\r",
            1,
            "network failed",
        )),
    );
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = observed.clone();
    assert!(matches!(
        block_on(NativeProjectPreparation.prepare(
            Arc::new(exec.clone()),
            create_input(Engine::Podman),
            CancelFlag::default(),
            Arc::new(move |update| recorded.lock().unwrap().push(update))
        )),
        Err(ProjectSetupError::CommandFailed { .. })
    ));
    assert!(
        observed
            .lock()
            .unwrap()
            .iter()
            .all(|update| update.phase != SetupPhase::CreatingContainer)
    );
    assert_eq!(exec.requests().len(), 1);
}
