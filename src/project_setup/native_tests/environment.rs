//! Disposable native environments exercise distro selection and non-root updates.
use super::*;
use nocterm_project_providers::{
    DistributionFamily, EnvironmentError, EnvironmentSource, WorkspaceEnvironment as _,
};

#[test]
#[ignore = "requires Docker and network access for Alpine packages"]
fn a_real_alpine_workspace_installs_distribution_languages() {
    distribution_languages("docker.io/library/alpine:3.22", DistributionFamily::Alpine);
}

#[test]
#[ignore = "requires Docker and network access for Arch packages"]
fn a_real_arch_workspace_bootstraps_and_manages_distribution_languages() {
    distribution_languages("docker.io/library/archlinux:base", DistributionFamily::Arch);
}

#[test]
#[ignore = "requires Docker and network access for Kali packages"]
fn a_real_kali_workspace_installs_distribution_languages() {
    distribution_languages(
        "docker.io/kalilinux/kali-rolling:latest",
        DistributionFamily::Debian,
    );
}

fn distribution_languages(image: &str, family: DistributionFamily) {
    let languages = [ProgrammingLanguage::Python, ProgrammingLanguage::Node]
        .into_iter()
        .map(LanguageSelection::default_for)
        .collect();
    let (runtime, exec, owned) = prepare_native(WorkspaceImage::Custom(image.into()), languages);
    let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let source =
            EnvironmentSource::host(owned.location().to_target().unwrap(), exec.clone()).unwrap();
        let environment = nocterm_project_runtime::NativeWorkspaceEnvironment::new(Arc::new(
            nocterm_project_runtime::NativeEnvironmentCommands,
        ));
        let before = block_on(environment.inspect(source.clone(), CancelFlag::default())).unwrap();
        assert_eq!(before.os.family, family, "{image}: {:?}", before.os);
        let selections = [ProgrammingLanguage::Python, ProgrammingLanguage::Node]
            .into_iter()
            .map(LanguageSelection::default_for)
            .collect();
        let after = block_on(environment.install(
            source.clone(),
            selections,
            CancelFlag::default(),
            Arc::new(|_| {}),
        ))
        .unwrap();
        for language in [ProgrammingLanguage::Python, ProgrammingLanguage::Node] {
            assert!(
                after
                    .tools
                    .iter()
                    .any(|tool| tool.language == language && tool.version.is_some()),
                "{image}: {after:?}"
            );
        }
        assert!(
            after
                .container
                .as_ref()
                .is_some_and(|container| container.image.is_some())
        );
        if family == DistributionFamily::Alpine {
            let result = block_on(environment.install(
                source.clone(),
                vec![LanguageSelection::parse(ProgrammingLanguage::Node, "22").unwrap()],
                CancelFlag::default(),
                Arc::new(|_| {}),
            ));
            assert!(
                matches!(result, Err(EnvironmentError::UnsupportedVersion { .. })),
                "{result:?}"
            );
            let preserved = block_on(environment.inspect(source, CancelFlag::default())).unwrap();
            assert_eq!(preserved.tools, after.tools);
        }
    }));
    let removed = block_on(runtime.discard_owned(exec, owned));
    checked.unwrap();
    removed.unwrap();
}

#[test]
#[ignore = "requires Docker and network access for an unprivileged Python installation"]
fn a_real_non_root_environment_updates_managed_tools_without_changing_system_python() {
    let (runtime, exec, owned) = prepare_native(
        WorkspaceImage::Custom("docker.io/library/ubuntu:24.04".into()),
        vec![],
    );
    let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        native_shell(
            &exec,
            &owned,
            "apt-get update >/dev/null && apt-get install -y --no-install-recommends ca-certificates curl xz-utils unzip python3 >/dev/null && useradd --create-home --uid 1001 nocterm-test",
        );
        let target = owned.location().to_target().unwrap();
        let user = Arc::new(ContainerUser {
            base: exec.clone(),
            container: owned.location().container().to_owned(),
        });
        let source = EnvironmentSource::scoped(target, user.clone(), Some(exec.clone())).unwrap();
        let environment = nocterm_project_runtime::NativeWorkspaceEnvironment::new(Arc::new(
            nocterm_project_runtime::NativeEnvironmentCommands,
        ));
        let before = block_on(environment.inspect(source.clone(), CancelFlag::default())).unwrap();
        assert_eq!(before.account.uid, 1001);
        let installed = block_on(environment.install(
            source.clone(),
            vec![LanguageSelection::parse(ProgrammingLanguage::Python, "3.13").unwrap()],
            CancelFlag::default(),
            Arc::new(|_| {}),
        ))
        .unwrap();
        assert!(
            installed
                .tools
                .iter()
                .any(|tool| tool.language == ProgrammingLanguage::Python
                    && tool
                        .version
                        .as_deref()
                        .is_some_and(|version| version.contains("3.13"))),
            "{installed:?}"
        );
        let fresh = block_on(async {
            user.exec(ExecRequest::new("sh").args(["-c", ". \"$HOME/.local/share/nocterm/toolchains/env.sh\"; python3 --version; /usr/bin/python3 --version"])).await.unwrap().collect(8192).await
        });
        assert!(fresh.exit.success(), "{}", fresh.text());
        assert!(fresh.text().contains("Python 3.13."));
        assert!(fresh.text().contains("Python 3.12."));
        let refreshed = block_on(environment.install(
            source,
            vec![LanguageSelection::parse(ProgrammingLanguage::Python, "3.13").unwrap()],
            CancelFlag::default(),
            Arc::new(|_| {}),
        ))
        .unwrap();
        assert_eq!(refreshed.account.uid, 1001);
    }));
    let removed = block_on(runtime.discard_owned(exec, owned));
    checked.unwrap();
    removed.unwrap();
}

pub(super) fn native_shell(
    exec: &Arc<nocterm_local::LocalExec>,
    owned: &OwnedContainer,
    script: &str,
) {
    let output = block_on(async {
        exec.exec(ExecRequest::new("docker").args([
            "exec",
            owned.location().container(),
            "sh",
            "-c",
            script,
        ]))
        .await
        .unwrap()
        .collect(32 * 1024)
        .await
    });
    assert!(
        output.exit.success(),
        "{}: {}",
        output.text(),
        output.exit.stderr
    );
}

/// A real container command source bound to a fixed unprivileged account.
struct ContainerUser {
    base: Arc<nocterm_local::LocalExec>,
    container: String,
}
impl nocterm_session::HostExec for ContainerUser {
    fn exec(
        &self,
        request: ExecRequest,
    ) -> nocterm_session::ExecFuture<nocterm_session::ExecOutput> {
        self.base.exec(
            ExecRequest::new("docker")
                .args([
                    "exec",
                    "--interactive",
                    "--user",
                    "1001",
                    "--env",
                    "HOME=/home/nocterm-test",
                    "--workdir",
                    "/home/nocterm-test",
                    &self.container,
                    "sh",
                    "-c",
                    &request.command_line(),
                ])
                .stdin(request.stdin.unwrap_or_default()),
        )
    }
}
