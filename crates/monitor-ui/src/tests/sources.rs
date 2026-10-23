//! Source selection and cancellation through the real shell projection.
use super::*;
use nocterm_project_model::{
    HostLocator, ProjectId, ProjectTarget, ProviderId, ProviderResourceRef, RootPath,
};
use nocterm_project_providers::{ContainerLocation, ContainerOrigin};
use nocterm_session::testing::{ExecResponse, FakeExec, Gate};
use nocterm_workspace::{
    ProjectSurface, SessionSpec,
    test_support::{ProjectSurfaceProbe, SessionProbe},
};

fn spec() -> SessionSpec {
    SessionSpec {
        options: Default::default(),
        title: "Inner".into(),
        profile: None,
        target: target("user@samehost"),
        auth: Default::default(),
        launch: None,
        credential: None,
        jumps: vec![],
    }
}
fn executor() -> FakeExec {
    let exec = FakeExec::default();
    exec.respond("uname", Ok(ExecResponse::text("Linux", 0, "")));
    exec
}
fn container(id: u8) -> ProjectTarget {
    // Engine identity is part of the checked provider reference, independent of host.
    ContainerLocation::new(
        HostLocator::Local,
        nocterm_containers::Engine::Docker,
        format!("{id:x}").repeat(64),
        RootPath::parse("/home").unwrap(),
        ContainerOrigin::Attached,
    )
    .unwrap()
    .to_target()
    .unwrap()
}
fn project(
    workspace: &Entity<Workspace>,
    id: u8,
    target: Option<ProjectTarget>,
    window: &mut Window,
    cx: &mut App,
) -> (Entity<ProjectSurfaceProbe>, ProjectSurface) {
    let factory = workspace.read(cx).project_surface_factory();
    let item = cx.new(|cx| {
        ProjectSurfaceProbe::new(factory, ProjectId::from_bytes([id; 16]), window, cx).unwrap()
    });
    let surface = item.read(cx).surface();
    if let Some(target) = target {
        surface.set_monitor_target(target, cx);
    }
    workspace.update(cx, |workspace, cx| {
        workspace
            .add_project_workspace(item.clone(), surface.clone(), window, cx)
            .unwrap()
    });
    (item, surface)
}
fn inner(
    surface: &ProjectSurface,
    exec: Option<&FakeExec>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<SessionProbe> {
    let item = SessionProbe::from_app(spec(), true, None, cx);
    if let Some(exec) = exec {
        item.update(cx, |item, cx| item.set_executor(Arc::new(exec.clone()), cx));
    }
    surface
        .add_item(surface.selection_receipt(cx), item.clone(), window, cx)
        .unwrap();
    item
}
fn frame(cpu: u64, memory: u64) -> String {
    format!(
        "@@cgroup\n0::/a\n@@mountinfo\n1 2 0:1 / /cg ro - cgroup2 cgroup rw\n@@selected\ncpu_path /cg/a\nmemory_path /cg/a\ncpu_version 2\nmemory_version 2\n@@container\nclock 10\ncpu_us {cpu}\nmemory {memory}\nlimit 100\n@@end\n"
    )
}
fn print(sink: &ExecSink, text: &str) {
    assert!(block_on(sink.send(text.as_bytes().to_vec())));
}
fn memory(monitor: &Entity<HostMonitor>, cx: &mut TestAppContext) -> u64 {
    monitor.read_with(cx, |monitor, _| {
        monitor.sampler().unwrap().snapshot().memory.unwrap().used
    })
}
#[gpui_kit::test]
fn selected_inner_container_history_never_uses_parent_or_unrelated_local_host(
    cx: &mut TestAppContext,
) {
    let local = executor();
    let local_sink = local.queue_stream("sh");
    let (window, workspace, monitor) = fixture(cx, Some(Arc::new(local.clone())));
    cx.run_until_parked();
    assert!(!local_sink.is_closed());
    let first = executor();
    let first_sink = first.queue_stream("sh");
    let (first_tab, first_surface) = cx
        .update_window(window, |_, window, cx| {
            let (tab, surface) = project(&workspace, 1, Some(container(1)), window, cx);
            inner(&surface, Some(&first), window, cx);
            (tab, surface)
        })
        .unwrap();
    cx.run_until_parked();
    assert!(local_sink.is_closed());
    print(&first_sink, &frame(10, 11));
    cx.run_until_parked();
    assert_eq!(memory(&monitor, cx), 11);
    let second = executor();
    let second_sink = second.queue_stream("sh");
    cx.update_window(window, |_, window, cx| {
        let (_, surface) = project(&workspace, 2, Some(container(2)), window, cx);
        inner(&surface, Some(&second), window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(first_sink.is_closed());
    print(&second_sink, &frame(20, 22));
    cx.run_until_parked();
    assert_eq!(memory(&monitor, cx), 22);
    let resumed = first.queue_stream("sh");
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.activate_item_by_id(first_tab.entity_id(), window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(memory(&monitor, cx), 11);
    assert!(second_sink.is_closed());
    assert!(!resumed.is_closed());
    assert_eq!(
        first
            .requests()
            .iter()
            .filter(|request| request.program == "uname")
            .count(),
        1
    );
    assert!(
        first
            .requests()
            .iter()
            .all(|request| request.program == "sh" || request.program == "uname")
    );
    assert_eq!(
        cx.update(|cx| workspace.read(cx).monitor_source(cx).target().cloned()),
        Some(container(1))
    );
    drop(first_surface);
}
#[gpui_kit::test]
fn empty_unknown_and_remote_without_execution_are_explicitly_unavailable(cx: &mut TestAppContext) {
    let local = executor();
    let sink = local.queue_stream("sh");
    let (window, workspace, monitor) = fixture(cx, Some(Arc::new(local.clone())));
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        project(&workspace, 1, Some(container(1)), window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(sink.is_closed());
    assert_eq!(status(cx, &monitor), Status::Offline);
    let unknown = ProjectTarget::Provider {
        provider: ProviderId::parse("external.unknown").unwrap(),
        resource_version: 1,
        reference: ProviderResourceRef::new(serde_json::json!({})).unwrap(),
    };
    cx.update_window(window, |_, window, cx| {
        let (_, surface) = project(&workspace, 2, Some(unknown), window, cx);
        inner(&surface, None, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(status(cx, &monitor), Status::Offline);
    assert_eq!(local.requests().len(), 1);
    open_session(
        cx,
        window,
        &workspace,
        SessionContext::new(target("user@without-exec"), None, true),
    );
    assert_eq!(status(cx, &monitor), Status::Offline);
    assert_eq!(local.requests().len(), 1);
}
#[gpui_kit::test]
fn hidden_completion_and_away_back_cannot_accept_an_old_platform_or_frame(cx: &mut TestAppContext) {
    let (window, workspace, monitor) = fixture(cx, None);
    let old = executor();
    let gate = Gate::pending();
    old.suspend(gate.clone());
    let old_sink = old.queue_stream("sh");
    let (tab, surface, item) = cx
        .update_window(window, |_, window, cx| {
            let (tab, surface) = project(&workspace, 1, Some(container(1)), window, cx);
            let item = inner(&surface, Some(&old), window, cx);
            (tab, surface, item)
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(status(cx, &monitor), Status::Detecting);
    let fresh = executor();
    let fresh_sink = fresh.queue_stream("sh");
    cx.update_window(window, |_, window, cx| {
        project(&workspace, 2, None, window, cx);
        item.update(cx, |item, cx| {
            item.set_executor(Arc::new(fresh.clone()), cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(status(cx, &monitor), Status::Offline);
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.activate_item_by_id(tab.entity_id(), window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    gate.release();
    cx.run_until_parked();
    assert!(
        old.requests()
            .iter()
            .all(|request| request.program == "uname")
    );
    print(&fresh_sink, &frame(20, 33));
    cx.run_until_parked();
    assert_eq!(status(cx, &monitor), Status::Running);
    assert_eq!(memory(&monitor, cx), 33);
    item.update(cx, |item, cx| item.set_connected(false, cx).unwrap());
    cx.run_until_parked();
    assert_eq!(status(cx, &monitor), Status::Offline);
    assert!(fresh_sink.is_closed());
    drop((old_sink, surface));
}

#[gpui_kit::test]
fn local_and_remote_folder_sources_require_their_actual_selected_authority(
    cx: &mut TestAppContext,
) {
    use crate::source::watched_source;
    use nocterm_workspace::MonitorSource;
    let (_, workspace, _) = fixture(cx, None);
    let surface = cx.update(|cx| workspace.read(cx).monitor_source(cx).surface());
    let settings = MonitorSettings::default();
    let local: Arc<dyn HostExec> = Arc::new(executor());
    let target = ProjectTarget::HostFolder {
        host: HostLocator::Local,
        root: RootPath::parse("/home").unwrap(),
    };
    let cached = SessionContext::new(super::target("user@previous-remote"), None, true)
        .with_exec(Some(Arc::new(executor())));
    let source = MonitorSource::project(surface, Some(target.clone()), Some(cached), true);
    assert!(source.session().is_none());
    assert!(Arc::ptr_eq(
        &watched_source(&source, Some(&local), &settings)
            .unwrap()
            .host()
            .exec,
        &local
    ));
    assert_eq!(
        watched_source(&source, Some(&local), &settings)
            .unwrap()
            .host()
            .key,
        HostKey::Local
    );
    let empty = MonitorSource::project(surface, Some(target.clone()), None, false);
    assert!(watched_source(&empty, Some(&local), &settings).is_none());
    let remote = executor();
    let session = SessionContext::new(super::target("user@folder"), None, true)
        .with_exec(Some(Arc::new(remote.clone())));
    let source = MonitorSource::project(surface, Some(target), Some(session), false);
    let watched = watched_source(&source, Some(&local), &settings).unwrap();
    assert!(!watched.container());
    assert!(matches!(watched.host().key, HostKey::Remote { .. }));
    assert!(remote.requests().is_empty());
}

#[gpui_kit::test]
fn closing_the_selected_container_restores_general_monitoring_and_cancels_its_watch(
    cx: &mut TestAppContext,
) {
    let local = executor();
    let initial = local.queue_stream("sh");
    let (window, workspace, monitor) = fixture(cx, Some(Arc::new(local.clone())));
    cx.run_until_parked();
    assert!(!initial.is_closed());
    let resource = executor();
    let resource_output = resource.queue_stream("sh");
    let tab = cx
        .update_window(window, |_, window, cx| {
            let (tab, surface) = project(&workspace, 1, Some(container(1)), window, cx);
            inner(&surface, Some(&resource), window, cx);
            tab
        })
        .unwrap();
    cx.run_until_parked();
    assert!(initial.is_closed());
    print(&resource_output, &frame(10, 11));
    cx.run_until_parked();
    assert_eq!(memory(&monitor, cx), 11);

    let resumed = local.queue_stream("sh");
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.close_tabs(
                tab.entity_id(),
                nocterm_workspace::TabCloseScope::Current,
                window,
                cx,
            );
        });
    })
    .unwrap();
    cx.run_until_parked();
    assert!(resource_output.is_closed());
    assert!(!resumed.is_closed());
    cx.update(|cx| {
        assert_eq!(
            workspace.read(cx).selected_context(cx),
            nocterm_project_model::ContextId::General
        )
    });
    print(
        &resumed,
        "@@meminfo\nMemTotal: 1000\nMemAvailable: 250\n@@end\n",
    );
    cx.run_until_parked();
    monitor.read_with(cx, |monitor, _| {
        let snapshot = monitor.sampler().unwrap().snapshot();
        assert_eq!(snapshot.memory.unwrap().percent(), 75.);
        assert!(snapshot.container.is_none());
    });
}

#[gpui_kit::test]
fn cpu_only_container_details_render_the_available_cpu_without_memory(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let (window, workspace, monitor, installed) = fixture_with_installation(cx, None);
    cx.update(|cx| {
        installed
            .settings()
            .update::<MonitorSettings>(cx, |settings| {
                settings.status_bar = vec![MonitorMetric::Cpu];
                settings.details = vec![MonitorMetric::Cpu];
            })
            .detach()
    });
    cx.run_until_parked();
    let resource = executor();
    let status_output = resource.queue_stream("sh");
    cx.update_window(window, |_, window, cx| {
        let (_, surface) = project(&workspace, 1, Some(container(1)), window, cx);
        inner(&surface, Some(&resource), window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let details_output = resource.queue_stream("sh");
    monitor.update(cx, |monitor, cx| monitor.set_open(true, cx));
    cx.run_until_parked();
    assert!(status_output.is_closed());
    let (details_window, _) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|cx| crate::details::DetailsView::new(monitor.clone(), cx))
        })
        .unwrap()
    });
    let first = frame(1000, 0)
        .lines()
        .filter(|line| !line.starts_with("memory") && !line.starts_with("limit "))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    print(&details_output, &first);
    cx.run_until_parked();
    cx.update_window(details_window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .try_find("host-monitor-section-CPU (one CPU)")
                .is_none()
        );
    })
    .unwrap();
    print(
        &details_output,
        &first
            .replace("clock 10", "clock 11")
            .replace("cpu_us 1000", "cpu_us 501000"),
    );
    cx.run_until_parked();
    cx.update_window(details_window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("host-monitor-section-CPU (one CPU)").label(),
            Some("CPU (one CPU)")
        );
        assert!(window.try_find("host-monitor-section-Memory").is_none());
    })
    .unwrap();
}
