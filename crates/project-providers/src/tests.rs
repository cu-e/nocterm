use super::*;
use nocterm_containers::Engine;
use nocterm_project_model::{HostLocator, ProjectTarget, RootPath};

#[test]
fn container_reference_round_trips_exact_engine_id_original_route_and_root() {
    for engine in Engine::ALL {
        let location = ContainerLocation::new(
            HostLocator::SshDirect {
                target: nocterm_session::Target::new("user", "host", 22),
                via: vec![nocterm_session::Target::new("jump", "bastion", 22)],
            },
            engine,
            "a".repeat(64),
            RootPath::parse("/project space").unwrap(),
            ContainerOrigin::Attached,
        )
        .unwrap();
        assert_eq!(
            ContainerLocation::from_target(&location.to_target().unwrap()).unwrap(),
            location
        );
    }
}
#[test]
fn unknown_engine_malformed_id_and_unsupported_version_are_typed_refusals() {
    assert!(matches!(
        ContainerLocation::new(
            HostLocator::Local,
            Engine::Docker,
            "short".into(),
            RootPath::parse("/").unwrap(),
            ContainerOrigin::Attached
        ),
        Err(ProjectSetupError::InvalidReference)
    ));
    let location = ContainerLocation::new(
        HostLocator::Local,
        Engine::Docker,
        "a".repeat(64),
        RootPath::parse("/").unwrap(),
        ContainerOrigin::Attached,
    )
    .unwrap();
    let mut target = location.to_target().unwrap();
    if let ProjectTarget::Provider {
        resource_version, ..
    } = &mut target
    {
        *resource_version = 99;
    }
    assert!(matches!(
        ContainerLocation::from_target(&target),
        Err(ProjectSetupError::UnsupportedVersion)
    ));
    let mut value = serde_json::to_value(location).unwrap();
    value["engine"] = "future".into();
    assert!(serde_json::from_value::<ContainerLocation>(value).is_err());
}
#[test]
fn repository_input_rejects_credentials_queries_controls_and_unsupported_protocols() {
    for value in [
        "https://user:password@example.test/repo",
        "https://host/repo?token=secret",
        "file:///etc",
        "--upload-pack=x",
        "https://host/repo\nnext",
    ] {
        assert!(GitRepository::parse(value.into()).is_err());
    }
    for value in [
        "https://host/team/repo.git",
        "git@host:team/repo.git",
        "ssh://git@host/team/repo.git",
    ] {
        assert!(GitRepository::parse(value.into()).is_ok());
    }
}
#[test]
fn codec_error_retains_original_concrete_source() {
    use std::error::Error as _;
    let error =
        ProjectSetupError::from(serde_json::from_str::<serde_json::Value>("{").unwrap_err());
    assert!(
        error
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<serde_json::Error>()
            .is_some()
    );
}

#[test]
fn language_versions_are_checked_per_language_and_defaults_are_truthful() {
    use ProgrammingLanguage::*;
    for (language, accepted) in [
        (Python, vec!["distribution", "3.13", "3.12.7"]),
        (Rust, vec!["stable", "1.90.0"]),
        (Bun, vec!["latest", "1.3.3"]),
        (Node, vec!["distribution", "22", "22.12.0"]),
        (Java, vec!["distribution", "21"]),
    ] {
        for value in accepted {
            let selected = LanguageSelection::parse(language, value).unwrap();
            assert_eq!(selected.language(), language);
            assert_eq!(selected.version().as_str(), value);
        }
        assert!(
            LanguageSelection::new(
                language,
                LanguageSelection::default_for(language).version().clone()
            )
            .is_ok()
        );
    }
    assert_eq!(
        LanguageSelection::default_for(Python).version(),
        &VersionChoice::DistributionDefault
    );
    assert_eq!(
        LanguageSelection::default_for(Rust).version(),
        &VersionChoice::Stable
    );
    assert_eq!(
        LanguageSelection::default_for(Bun).version(),
        &VersionChoice::Latest
    );
}
#[test]
fn incompatible_ambiguous_and_shell_fragment_versions_are_rejected() {
    use ProgrammingLanguage::*;
    for (language, value) in [
        (Python, "stable"),
        (Rust, "distribution"),
        (Bun, "distribution"),
        (Java, "21.0.1"),
        (Rust, "1.90"),
        (Node, "22.2"),
        (Bun, "1.3"),
        (Python, "2.7"),
        (Python, "3.013"),
        (Node, "22;touch /tmp/nope"),
        (Java, "-21"),
        (Python, " 3.13"),
        (Bun, "1.3.3\n"),
        (Node, ""),
        (Java, "0"),
    ] {
        assert!(
            matches!(
                LanguageSelection::parse(language, value),
                Err(ProjectSetupError::InvalidInput(_))
            ),
            "{language:?} {value}"
        );
    }
}
#[test]
fn duplicate_languages_are_refused_and_checked_custom_image_languages_are_allowed() {
    let mut request = ProjectSetupRequest {
        project: nocterm_project_model::ProjectId::from_bytes([1; 16]),
        host: HostLocator::Local,
        mode: SetupMode::Create {
            engine: Engine::Docker,
            image: WorkspaceImage::Ubuntu,
            languages: vec![LanguageSelection::default_for(ProgrammingLanguage::Python); 2],
            root: None,
        },
        repository: None,
    };
    assert!(matches!(
        request.validate(),
        Err(ProjectSetupError::InvalidInput(_))
    ));
    request.mode = SetupMode::Create {
        engine: Engine::Docker,
        image: WorkspaceImage::Custom("debian:13".into()),
        languages: vec![LanguageSelection::parse(ProgrammingLanguage::Java, "21").unwrap()],
        root: None,
    };
    assert!(request.validate().is_ok());
}
#[test]
fn image_queries_reject_options_controls_short_and_excessive_input() {
    for query in ["", "x", "  ", "--all", "de\nbi", "de\0bi", &"a".repeat(257)] {
        assert!(matches!(
            ImageSearchQuery::parse(query),
            Err(ImageSearchError::InvalidQuery)
        ));
    }
    assert_eq!(
        ImageSearchQuery::parse(" quay.io/team ").unwrap().as_str(),
        "quay.io/team"
    );
}

#[test]
fn environment_sources_keep_execution_scope_and_group_profile_aliases_independently_of_folder() {
    use nocterm_session::{HostExec, Target, testing::FakeExec};
    use std::sync::Arc;
    let exec: Arc<dyn HostExec> = Arc::new(FakeExec::default());
    let target = Target::new("dev", "server", 22);
    let via = vec![Target::new("jump", "bastion", 22)];
    let host = HostLocator::Ssh {
        profile: "first".into(),
        target: target.clone(),
        via: via.clone(),
    };
    let source = EnvironmentSource::host(
        ProjectTarget::HostFolder {
            host,
            root: RootPath::parse("/first").unwrap(),
        },
        exec.clone(),
    )
    .unwrap();
    let alias = EnvironmentSource::scoped(
        ProjectTarget::HostFolder {
            host: HostLocator::Ssh {
                profile: "second".into(),
                target: target.clone(),
                via: via.clone(),
            },
            root: RootPath::parse("/second").unwrap(),
        },
        exec.clone(),
        None,
    )
    .unwrap();
    let direct = EnvironmentSource::host(
        ProjectTarget::HostFolder {
            host: HostLocator::SshDirect { target, via },
            root: RootPath::parse("/third").unwrap(),
        },
        exec.clone(),
    )
    .unwrap();
    assert_eq!(source.key(), alias.key());
    assert_eq!(source.key(), direct.key());
    assert!(
        matches!(alias.execution(),EnvironmentExecution::Scoped(actual) if Arc::ptr_eq(actual,&exec))
    );
    assert!(alias.host_metadata().is_none());
    assert!(
        matches!(source.execution(),EnvironmentExecution::Host(actual) if Arc::ptr_eq(actual,&exec))
    );
}
#[test]
fn unsupported_provider_sources_never_gain_execution_authority() {
    let target = ProjectTarget::Provider {
        provider: nocterm_project_model::ProviderId::parse("future.provider").unwrap(),
        resource_version: 1,
        reference: nocterm_project_model::ProviderResourceRef::new(serde_json::json!({})).unwrap(),
    };
    assert!(matches!(
        EnvironmentSource::host(
            target,
            std::sync::Arc::new(nocterm_session::testing::FakeExec::default())
        ),
        Err(EnvironmentError::Unavailable)
    ));
}
#[test]
fn transfer_progress_bounds_labels_and_rejects_an_invalid_denominator() {
    let transfer = TransferProgress::new(Some(&format!("bad\r\n{}", "x".repeat(256))), 12, Some(5));
    assert_eq!(transfer.current_bytes, 12);
    assert_eq!(transfer.total_bytes, None);
    let subject = transfer.subject.unwrap();
    assert_eq!(subject.len(), 128);
    assert!(!subject.contains(['\r', '\n']));
    let update = SetupUpdate::phase(SetupPhase::PullingImage);
    assert!(update.transfer.is_none());
    assert_eq!(update.label(), "Pulling image");
}

#[test]
fn removal_source_requires_the_exact_original_host_and_ordered_route() {
    use std::sync::Arc;
    let host = HostLocator::SshDirect {
        target: nocterm_session::Target::new("dev", "server.test", 22),
        via: vec![
            nocterm_session::Target::new("jump", "first.test", 22),
            nocterm_session::Target::new("jump", "second.test", 22),
        ],
    };
    let location = ContainerLocation::new(
        host.clone(),
        nocterm_containers::Engine::Docker,
        "a".repeat(64),
        RootPath::parse("/home").unwrap(),
        ContainerOrigin::Attached,
    )
    .unwrap();
    let mut reordered = host.clone();
    if let HostLocator::SshDirect { via, .. } = &mut reordered {
        via.reverse();
    }
    let exec = Arc::new(nocterm_session::testing::FakeExec::default());
    assert!(matches!(
        ContainerActionSource::new(location.clone(), &HostLocator::Local, exec.clone()),
        Err(ContainerActionError::StaleSource)
    ));
    assert!(matches!(
        ContainerActionSource::new(location.clone(), &reordered, exec.clone()),
        Err(ContainerActionError::StaleSource)
    ));
    assert!(exec.requests().is_empty());
    assert!(ContainerActionSource::new(location, &host, exec).is_ok());
}
#[test]
fn removal_errors_preserve_original_codec_causes() {
    let source = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
    let error = ContainerActionError::Runtime(ProjectSetupError::from(source));
    let mut cause = std::error::Error::source(&error);
    let mut found = false;
    while let Some(error) = cause {
        found |= error.downcast_ref::<serde_json::Error>().is_some();
        cause = error.source();
    }
    assert!(found);
}

#[test]
fn scoped_requests_preserve_literal_arguments_stdin_engine_id_and_optional_cwd() {
    for engine in Engine::ALL {
        let location = ContainerLocation::new(
            HostLocator::Local,
            engine,
            "b".repeat(64),
            RootPath::parse("/root space").unwrap(),
            ContainerOrigin::Attached,
        )
        .unwrap();
        for cwd in [false, true] {
            let mut request =
                nocterm_session::ExecRequest::new("printf").args(["%s", "literal $(unchanged)"]);
            request.stdin = Some(vec![0, 1, 255]);
            let wrapped = location.exec_request(request, cwd);
            let mut expected = vec!["exec", "--interactive"];
            if cwd {
                expected.extend(["--workdir", "/root space"]);
            }
            expected.extend([location.container(), "printf", "%s", "literal $(unchanged)"]);
            assert_eq!(wrapped.program, engine.command().program);
            assert_eq!(wrapped.args, expected);
            assert_eq!(wrapped.stdin, Some(vec![0, 1, 255]));
        }
    }
}
