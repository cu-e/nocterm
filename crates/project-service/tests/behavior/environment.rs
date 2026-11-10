//! Environment admission retains work without changing workspace metadata or resources.
use std::sync::Arc;

use futures::{FutureExt as _, future::BoxFuture};
use nocterm_containers::Engine;
use nocterm_project_model::{Catalog, HostLocator, ProjectTarget, RootPath};
use nocterm_project_providers::{
    ContainerLocation, ContainerOrigin, DistributionFamily, EnvironmentAccount, EnvironmentError,
    EnvironmentOs, EnvironmentSnapshot, EnvironmentSource, LanguageSelection, LibcFamily,
    ProgrammingLanguage, SetupPhase, SetupUpdate, ToolObservation,
    testing::FakeWorkspaceEnvironment,
};
use nocterm_project_service::{EnvironmentOperation, PreparedProjects, ProjectsHandle};
use nocterm_session::{
    Target,
    testing::{FakeExec, Gate},
};
use nocterm_store::ShutdownCoordinator;

use crate::support::*;

fn source(profile: &str, root: &str, engine: Option<Engine>) -> EnvironmentSource {
    let host = HostLocator::Ssh {
        profile: profile.into(),
        target: Target::new("dev", "workspace.example", 22),
        via: vec![Target::new("jump", "bastion.example", 22)],
    };
    let root = RootPath::parse(root).unwrap();
    let target = if let Some(engine) = engine {
        ContainerLocation::new(
            host,
            engine,
            "a".repeat(64),
            root,
            ContainerOrigin::Attached,
        )
        .unwrap()
        .to_target()
        .unwrap()
    } else {
        ProjectTarget::HostFolder { host, root }
    };
    EnvironmentSource::host(target, Arc::new(FakeExec::default())).unwrap()
}

fn snapshot(version: &str) -> EnvironmentSnapshot {
    EnvironmentSnapshot {
        os: EnvironmentOs {
            kernel: "Linux".into(),
            id: "debian".into(),
            name: "Debian GNU/Linux".into(),
            release: "13".into(),
            architecture: "x86_64".into(),
            libc: LibcFamily::Glibc,
            family: DistributionFamily::Debian,
        },
        account: EnvironmentAccount {
            user: "dev".into(),
            uid: 1000,
            home: "/home/dev".into(),
            writable_home: true,
        },
        tools: vec![ToolObservation {
            language: ProgrammingLanguage::Python,
            version: Some(version.into()),
        }],
        container: None,
        available_programs: vec!["curl".into(), "tar".into()],
    }
}

fn selections() -> Vec<LanguageSelection> {
    vec![LanguageSelection::parse(ProgrammingLanguage::Python, "3.13").unwrap()]
}

fn suspended(gate: &Gate) -> BoxFuture<'static, ()> {
    let gate = gate.clone();
    async move { gate.wait().await }.boxed()
}

fn fixture(shutdown: &ShutdownCoordinator, cx: &mut gpui::TestAppContext) -> ProjectsHandle {
    install(
        PreparedProjects::loaded(Ok(Some(Catalog::new(vec![definition(1)]).unwrap())), false),
        ready_writer(),
        shutdown,
        cx,
    )
}

fn operation(
    handle: &ProjectsHandle,
    port: &FakeWorkspaceEnvironment,
    source: EnvironmentSource,
    cx: &mut gpui::TestAppContext,
) -> EnvironmentOperation {
    cx.update(|cx| handle.environment_operation(Arc::new(port.clone()), source, cx))
        .unwrap()
}

#[gpui::test]
async fn installs_serialize_across_folder_and_saved_profile_aliases(cx: &mut gpui::TestAppContext) {
    let handle = fixture(&ShutdownCoordinator::default(), cx);
    for engine in [None, Some(Engine::Docker), Some(Engine::Podman)] {
        let port = FakeWorkspaceEnvironment::default();
        let gate = Gate::pending();
        port.install_after(suspended(&gate), Ok(snapshot("3.13")))
            .unwrap();
        port.respond_install(Ok(snapshot("3.13"))).unwrap();
        let first = operation(&handle, &port, source("first", "/one", engine), cx);
        let alias = operation(&handle, &port, source("alias", "/two", engine), cx);
        let pending = first.install(selections(), Arc::new(|_| {}));
        cx.run_until_parked();
        assert!(matches!(
            alias.install(selections(), Arc::new(|_| {})).await,
            Err(EnvironmentError::Busy)
        ));
        assert_eq!(port.calls().unwrap().len(), 1);
        gate.release();
        pending.await.unwrap();
        assert_eq!(
            alias.install(selections(), Arc::new(|_| {})).await.unwrap(),
            snapshot("3.13")
        );
    }
    cx.update(|cx| assert_eq!(handle.catalog(cx).get(project(1)), Some(&definition(1))));
}

#[gpui::test]
async fn finishing_an_inspection_does_not_release_another_installation(
    cx: &mut gpui::TestAppContext,
) {
    let handle = fixture(&ShutdownCoordinator::default(), cx);
    let port = FakeWorkspaceEnvironment::default();
    let gate = Gate::pending();
    port.install_after(suspended(&gate), Ok(snapshot("3.13")))
        .unwrap();
    port.respond_install(Ok(snapshot("3.13"))).unwrap();
    port.respond_inspect(Ok(snapshot("3.12"))).unwrap();
    let first = operation(&handle, &port, source("first", "/one", None), cx);
    let observer = operation(&handle, &port, source("alias", "/two", None), cx);
    let pending = first.install(selections(), Arc::new(|_| {}));
    cx.run_until_parked();
    assert_eq!(observer.inspect().await.unwrap(), snapshot("3.12"));
    assert!(matches!(
        observer.install(selections(), Arc::new(|_| {})).await,
        Err(EnvironmentError::Busy)
    ));
    gate.release();
    pending.await.unwrap();
    observer
        .install(selections(), Arc::new(|_| {}))
        .await
        .unwrap();
}

#[gpui::test]
async fn dropping_the_result_receiver_keeps_work_retained_and_latest_progress_visible(
    cx: &mut gpui::TestAppContext,
) {
    let handle = fixture(&ShutdownCoordinator::default(), cx);
    let port = FakeWorkspaceEnvironment::default();
    let gate = Gate::pending();
    let latest = SetupUpdate::phase(SetupPhase::InstallingTools);
    port.progress(vec![
        SetupUpdate::phase(SetupPhase::Inspecting),
        latest.clone(),
    ])
    .unwrap();
    port.install_after(suspended(&gate), Ok(snapshot("3.13")))
        .unwrap();
    let job = operation(
        &handle,
        &port,
        source("first", "/one", Some(Engine::Docker)),
        cx,
    );
    drop(job.install(selections(), Arc::new(|_| {})));
    cx.run_until_parked();
    assert!(job.is_running());
    assert_eq!(job.progress(), Some(latest));
    gate.release();
    cx.run_until_parked();
    assert!(!job.is_running());
    assert_eq!(job.snapshot(), Some(snapshot("3.13")));
    assert!(job.progress().is_none());
    cx.update(|cx| assert_eq!(handle.catalog(cx).get(project(1)), Some(&definition(1))));
}

#[gpui::test]
async fn canceled_installation_refuses_late_success_and_keeps_observed_versions(
    cx: &mut gpui::TestAppContext,
) {
    let handle = fixture(&ShutdownCoordinator::default(), cx);
    let port = FakeWorkspaceEnvironment::default();
    let gate = Gate::pending();
    port.respond_inspect(Ok(snapshot("3.12"))).unwrap();
    port.install_after(suspended(&gate), Ok(snapshot("3.13")))
        .unwrap();
    let job = operation(
        &handle,
        &port,
        source("first", "/one", Some(Engine::Podman)),
        cx,
    );
    job.inspect().await.unwrap();
    let mut pending = job.install(selections(), Arc::new(|_| {}));
    cx.run_until_parked();
    job.cancel();
    assert!(port.calls().unwrap()[1].cancel.is_cancelled());
    assert!(pending.as_mut().now_or_never().is_none());
    gate.release();
    assert!(matches!(pending.await, Err(EnvironmentError::Canceled)));
    assert_eq!(job.snapshot(), Some(snapshot("3.12")));
    assert!(matches!(job.error(), Some(EnvironmentError::Canceled)));
    assert!(matches!(
        job.inspect().await,
        Err(EnvironmentError::Canceled)
    ));
    assert_eq!(port.calls().unwrap().len(), 2);
    cx.update(|cx| assert_eq!(handle.catalog(cx).get(project(1)), Some(&definition(1))));
}

#[gpui::test]
async fn partial_install_failure_preserves_observations_and_allows_explicit_retry(
    cx: &mut gpui::TestAppContext,
) {
    let handle = fixture(&ShutdownCoordinator::default(), cx);
    let port = FakeWorkspaceEnvironment::default();
    port.respond_inspect(Ok(snapshot("3.12"))).unwrap();
    port.respond_install(Err(EnvironmentError::PartialInstall {
        completed: vec![ProgrammingLanguage::Python],
        cause: Box::new(EnvironmentError::ReadOnlyHome),
    }))
    .unwrap();
    port.respond_install(Ok(snapshot("3.13"))).unwrap();
    let job = operation(&handle, &port, source("first", "/one", None), cx);
    job.inspect().await.unwrap();
    assert!(
        matches!(job.install(selections(), Arc::new(|_| {})).await, Err(EnvironmentError::PartialInstall { completed, .. }) if completed == vec![ProgrammingLanguage::Python])
    );
    assert_eq!(job.snapshot(), Some(snapshot("3.12")));
    assert!(matches!(
        job.error(),
        Some(EnvironmentError::PartialInstall { .. })
    ));
    assert_eq!(
        job.install(selections(), Arc::new(|_| {})).await.unwrap(),
        snapshot("3.13")
    );
    assert!(job.error().is_none());
    cx.update(|cx| assert_eq!(handle.catalog(cx).get(project(1)), Some(&definition(1))));
}

#[gpui::test]
async fn shutdown_cancels_and_drains_work_after_all_external_receivers_are_dropped(
    cx: &mut gpui::TestAppContext,
) {
    let shutdown = ShutdownCoordinator::default();
    let handle = fixture(&shutdown, cx);
    let port = FakeWorkspaceEnvironment::default();
    let gate = Gate::pending();
    port.install_after(suspended(&gate), Ok(snapshot("3.13")))
        .unwrap();
    let source = source("first", "/one", Some(Engine::Docker));
    let job = operation(&handle, &port, source.clone(), cx);
    drop(job.install(selections(), Arc::new(|_| {})));
    cx.run_until_parked();
    drop(job);
    let work = cx.update(|cx| shutdown.capture_all(cx));
    assert!(port.calls().unwrap()[0].cancel.is_cancelled());
    assert!(matches!(
        cx.update(|cx| handle.environment_operation(Arc::new(port.clone()), source, cx)),
        Err(EnvironmentError::Unavailable)
    ));
    let mut drain = Box::pin(cx.background_executor.spawn(work));
    cx.run_until_parked();
    assert!(drain.as_mut().now_or_never().is_none());
    gate.release();
    let report = drain.await;
    assert!(!report.timed_out());
    assert!(report.failures().is_empty());
    assert_eq!(port.calls().unwrap().len(), 1);
    cx.update(|cx| assert_eq!(handle.catalog(cx).get(project(1)), Some(&definition(1))));
}
