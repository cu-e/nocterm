use super::*;
use futures::executor::block_on;
use nocterm_containers::Engine;
use nocterm_project_environment::NativeWorkspaceEnvironment;
use nocterm_project_model::{HostLocator, ProjectTarget, RootPath};
use nocterm_project_providers::{
    ContainerLocation, ContainerOrigin, LibcFamily, ProgrammingLanguage,
};
use nocterm_session::testing::{ExecResponse, FakeExec};
fn target() -> ProjectTarget {
    ProjectTarget::HostFolder {
        host: HostLocator::Local,
        root: RootPath::parse("/workspace").unwrap(),
    }
}
fn response(exec: &FakeExec, prefix: &str, output: String) {
    exec.respond(prefix, Ok(ExecResponse::text(output, 0, "")));
}
fn install(
    exec: FakeExec,
    selections: Vec<LanguageSelection>,
) -> Result<EnvironmentSnapshot, EnvironmentError> {
    block_on(
        NativeWorkspaceEnvironment::new(Arc::new(
            nocterm_project_runtime::NativeEnvironmentCommands,
        ))
        .install(
            EnvironmentSource::host(target(), Arc::new(exec)).unwrap(),
            selections,
            CancelFlag::default(),
            Arc::new(|_| {}),
        ),
    )
}
#[test]
fn host_and_already_scoped_sources_route_exactly_once_and_probe_actual_container_os() {
    for scoped in [false, true] {
        let exec = FakeExec::default();
        response(
            &exec,
            if scoped { "sh" } else { "podman exec" },
            probe_fixture()
                .replace("ID=ubuntu", "ID=alpine")
                .replace("glibc 2.39", "musl libc"),
        );
        let location = ContainerLocation::new(
            HostLocator::Local,
            Engine::Podman,
            "a".repeat(64),
            RootPath::parse("/workspace").unwrap(),
            ContainerOrigin::Attached,
        )
        .unwrap();
        let target = location.to_target().unwrap();
        let source = if scoped {
            EnvironmentSource::scoped(target, Arc::new(exec.clone()), None).unwrap()
        } else {
            EnvironmentSource::host(target, Arc::new(exec.clone())).unwrap()
        };
        let snapshot = block_on(
            NativeWorkspaceEnvironment::new(Arc::new(
                nocterm_project_runtime::NativeEnvironmentCommands,
            ))
            .inspect(source, CancelFlag::default()),
        )
        .unwrap();
        assert_eq!(snapshot.os.id, "alpine");
        assert_eq!(snapshot.os.libc, LibcFamily::Musl);
        assert_eq!(snapshot.container.unwrap().id, "a".repeat(64));
        let requests = exec.requests();
        assert_eq!(requests[0].program, if scoped { "sh" } else { "podman" });
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.args.first().is_some_and(|arg| arg == "exec"))
                .count(),
            usize::from(!scoped)
        );
    }
}
#[test]
fn unsupported_versions_read_only_home_and_missing_prerequisites_refuse_before_mutation() {
    for (fixture, expected) in [
        (probe_fixture().replace("glibc 2.39", "musl libc"), "musl"),
        (
            probe_fixture().replace("writable\x00yes", "writable\x00no"),
            "home",
        ),
        (
            probe_fixture()
                .replace("uid\x000", "uid\x001000")
                .replace("program\x00unzip\x00", ""),
            "prerequisite",
        ),
    ] {
        let exec = FakeExec::default();
        response(&exec, "sh", fixture);
        let error = install(
            exec.clone(),
            vec![LanguageSelection::parse(ProgrammingLanguage::Node, "22").unwrap()],
        )
        .unwrap_err();
        match expected {
            "musl" => assert!(matches!(error, EnvironmentError::UnsupportedVersion { .. })),
            "home" => assert!(matches!(error, EnvironmentError::ReadOnlyHome)),
            _ => assert!(matches!(
                error,
                EnvironmentError::MissingPrerequisite {
                    program: "unzip",
                    ..
                }
            )),
        }
        assert_eq!(exec.requests().len(), 1);
    }
}
#[test]
fn distro_package_strategies_follow_observed_os_without_sudo_or_unsupported_abi_fallback() {
    for (id, manager) in [("kali", "apt-get"), ("alpine", "apk"), ("arch", "pacman")] {
        let exec = FakeExec::default();
        let mut fixture = probe_fixture()
            .replace("ID=ubuntu", &format!("ID={id}"))
            .replace("program\x00apt-get", &format!("program\x00{manager}"));
        if id == "arch" {
            fixture.push_str("program\x00pacman-database\x00");
        }
        response(&exec, "sh", fixture);
        response(&exec, manager, String::new());
        install(
            exec.clone(),
            vec![LanguageSelection::default_for(ProgrammingLanguage::Node)],
        )
        .unwrap();
        let requests = exec.requests();
        assert!(
            requests
                .iter()
                .any(|request| request.args.iter().any(|argument| argument == manager))
        );
        assert!(requests.iter().all(|request| request.program != "sudo"));
        assert!(
            requests
                .iter()
                .all(|request| !request.args.iter().any(|argument| argument == "-Sy"))
        );
    }
}
#[test]
fn partial_install_failure_reports_completed_tools_and_never_deletes_existing_environment() {
    let exec = FakeExec::default();
    exec.respond(
        "sh -s -- /root 1.90.0",
        Ok(ExecResponse::text("", 1, "upstream unavailable")),
    );
    response(&exec, "sh", probe_fixture());
    response(&exec, "apt-get", String::new());
    let selections = vec![
        LanguageSelection::parse(ProgrammingLanguage::Python, "3.13").unwrap(),
        LanguageSelection::parse(ProgrammingLanguage::Rust, "1.90.0").unwrap(),
    ];
    assert!(
        matches!(install(exec.clone(),selections),Err(EnvironmentError::PartialInstall {completed,..}) if completed==[ProgrammingLanguage::Python])
    );
    assert!(exec.requests().iter().all(|request| {
        request.program != "rm"
            && !request
                .args
                .first()
                .is_some_and(|argument| argument == "rm")
    }));
}
#[test]
fn duplicate_install_selection_is_refused_without_starting_a_probe() {
    let exec = FakeExec::default();
    let selection = LanguageSelection::default_for(ProgrammingLanguage::Rust);
    assert!(matches!(
        install(exec.clone(), vec![selection.clone(), selection]),
        Err(EnvironmentError::InvalidInput(_))
    ));
    assert!(exec.requests().is_empty());
}

#[test]
fn existing_arch_without_package_databases_requires_an_explicit_administrator_update() {
    let exec = FakeExec::default();
    response(
        &exec,
        "sh",
        probe_fixture()
            .replace("ID=ubuntu", "ID=arch")
            .replace("program\x00apt-get", "program\x00pacman"),
    );
    let error = install(
        exec.clone(),
        vec![LanguageSelection::default_for(ProgrammingLanguage::Node)],
    )
    .unwrap_err();
    assert!(matches!(
        error,
        EnvironmentError::MissingPrerequisite {
            program: "pacman package databases",
            ..
        }
    ));
    assert_eq!(exec.requests().len(), 1);
}
#[test]
fn freshly_owned_arch_can_initialize_package_databases_without_upgrading_an_existing_environment() {
    let exec = FakeExec::default();
    response(
        &exec,
        &format!("podman exec --interactive {} sh -c", super::id()),
        probe_fixture()
            .replace("ID=ubuntu", "ID=arch")
            .replace("program\x00apt-get", "program\x00pacman"),
    );
    super::answer(&exec, "podman create", super::id());
    super::answer(&exec, "podman", String::new());
    super::prepare(
        exec.clone(),
        super::request(nocterm_project_providers::SetupMode::Create {
            engine: Engine::Podman,
            image: nocterm_project_providers::WorkspaceImage::Custom("archlinux:base".into()),
            languages: vec![LanguageSelection::default_for(ProgrammingLanguage::Node)],
            root: None,
        }),
    )
    .unwrap();
    assert!(
        exec.requests()
            .iter()
            .any(|request| request.program == "podman"
                && request.args.iter().any(|argument| argument == "-Syu"))
    );
    assert!(
        exec.requests()
            .iter()
            .all(|request| request.program != "pacman")
    );
}
