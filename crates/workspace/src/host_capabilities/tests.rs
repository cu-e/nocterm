//! The workspace observation follows the active context and the active tab's session.
use gpui_kit::{AppContext as _, Entity, TestAppContext, Window, WindowOptions};
use nocterm_plugin_api::{
    HostError,
    capability::workspace::{WorkspaceKind, WorkspaceState},
};
use nocterm_plugin_model::PluginId;
use nocterm_plugin_ports::{Observe as _, PluginProviders};
use nocterm_project_model::ProjectId;
use nocterm_session::{Auth, Target};

use super::ShellCapabilities;
use crate::{
    ProjectSurface, SessionSpec, Workspace,
    test_support::{ProjectSurfaceProbe, SessionProbe},
};

fn plugin() -> PluginId {
    PluginId::user("acme", "sample").expect("a valid plugin id")
}

fn window(cx: &mut TestAppContext) -> (gpui_kit::AnyWindowHandle, Entity<Workspace>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(nocterm_ui::Design::new(nocterm_ui::DesignTokens::builtin()));
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                let deps = crate::Deps::new(nocterm_ui::LayoutHandle::in_memory(cx));
                Workspace::new(deps, window, cx)
            })
        })
        .expect("a window opens")
    })
}

fn spec() -> SessionSpec {
    SessionSpec {
        options: Default::default(),
        title: "vm".into(),
        profile: None,
        target: Target::new("root", "vm", 22),
        auth: Auth::Password,
        launch: None,
        credential: None,
        jumps: Vec::new(),
    }
}

fn add_project(
    workspace: &Entity<Workspace>,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) -> ProjectSurface {
    let factory = workspace.read(cx).project_surface_factory();
    let tab = cx.new(|cx| {
        ProjectSurfaceProbe::new(factory, ProjectId::from_bytes([7; 16]), window, cx)
            .expect("the project surface opens")
    });
    let surface = tab.read(cx).surface();
    workspace.update(cx, |workspace, cx| {
        workspace
            .add_project_workspace(tab, surface.clone(), window, cx)
            .expect("the project tab is added")
    });
    surface
}

/// The provider of a window, with its first refresh done.
fn follow(workspace: &Entity<Workspace>, cx: &mut TestAppContext) -> ShellCapabilities {
    let provider = cx.update(|cx| ShellCapabilities::new(workspace, cx));
    cx.run_until_parked();
    provider
}

fn observed(provider: &ShellCapabilities, cx: &gpui_kit::App) -> Result<WorkspaceState, HostError> {
    provider.sample(&plugin(), cx)
}

#[gpui_kit::test]
fn an_empty_window_is_the_local_machine(cx: &mut TestAppContext) {
    let (_, workspace) = window(cx);
    let provider = follow(&workspace, cx);
    cx.update(|cx| {
        assert_eq!(
            observed(&provider, cx),
            Ok(WorkspaceState {
                kind: WorkspaceKind::Local,
                connected: true
            })
        );
    });
}

#[gpui_kit::test]
fn a_session_tab_is_remote_and_follows_its_connection(cx: &mut TestAppContext) {
    let (window, workspace) = window(cx);
    let provider = follow(&workspace, cx);
    let session = cx
        .update_window(window, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                let session = SessionProbe::new(spec(), true, cx);
                workspace.add_item(session.clone(), window, cx);
                session
            })
        })
        .expect("the window is open");
    cx.update(|cx| {
        assert_eq!(
            observed(&provider, cx),
            Ok(WorkspaceState {
                kind: WorkspaceKind::Remote,
                connected: true
            })
        );
    });
    cx.update(|cx| {
        session
            .update(cx, |probe, cx| probe.set_connected(false, cx))
            .expect("the probe changes state");
    });
    cx.update(|cx| {
        assert_eq!(
            observed(&provider, cx),
            Ok(WorkspaceState {
                kind: WorkspaceKind::Remote,
                connected: false
            })
        );
    });
}

#[gpui_kit::test]
fn a_selected_project_is_a_folder_and_leaving_it_restores_the_session(cx: &mut TestAppContext) {
    let (window, workspace) = window(cx);
    let provider = follow(&workspace, cx);
    let session = cx
        .update_window(window, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                let session = SessionProbe::new(spec(), true, cx);
                workspace.add_item(session.clone(), window, cx);
                session
            })
        })
        .expect("the window is open");
    cx.update_window(window, |_, window, cx| {
        add_project(&workspace, window, cx);
    })
    .expect("the window is open");
    cx.update(|cx| {
        assert_eq!(
            observed(&provider, cx).map(|state| state.kind),
            Ok(WorkspaceKind::Folder)
        );
    });
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.activate_item_by_id(session.entity_id(), window, cx);
        });
    })
    .expect("the window is open");
    cx.update(|cx| {
        assert_eq!(
            observed(&provider, cx).map(|state| state.kind),
            Ok(WorkspaceKind::Remote)
        );
    });
}

#[gpui_kit::test]
fn the_provider_can_be_built_and_sampled_from_inside_a_workspace_update(cx: &mut TestAppContext) {
    let (window, workspace) = window(cx);
    let built = cx
        .update_window(window, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                let provider = ShellCapabilities::new(&cx.entity(), cx);
                // Sampling here would panic if it read the workspace, which is updating.
                assert_eq!(provider.sample(&plugin(), cx), Err(HostError::Unavailable));
                let session = SessionProbe::new(spec(), true, cx);
                workspace.add_item(session, window, cx);
                assert!(provider.sample(&plugin(), cx).is_err());
                provider
            })
        })
        .expect("the window is open");
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            observed(&built, cx).map(|state| state.kind),
            Ok(WorkspaceKind::Remote)
        );
    });
}

#[gpui_kit::test]
fn sampling_from_inside_an_update_keeps_the_last_state(cx: &mut TestAppContext) {
    let (window, workspace) = window(cx);
    let provider = follow(&workspace, cx);
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            let session = SessionProbe::new(spec(), true, cx);
            workspace.add_item(session, window, cx);
            assert_eq!(
                provider.sample(&plugin(), cx).map(|state| state.kind),
                Ok(WorkspaceKind::Local),
                "the change is reported once the update has ended"
            );
        });
    })
    .expect("the window is open");
    cx.update(|cx| {
        assert_eq!(
            observed(&provider, cx).map(|state| state.kind),
            Ok(WorkspaceKind::Remote)
        );
    });
}

#[gpui_kit::test]
fn a_closed_window_is_unavailable_through_the_providers(cx: &mut TestAppContext) {
    let (window, workspace) = window(cx);
    let provider = follow(&workspace, cx);
    let providers = PluginProviders::new().with_workspace(std::rc::Rc::new(provider));
    cx.update(|cx| {
        assert!(providers.read("workspace.kind", &plugin(), cx).is_ok());
    });
    drop(workspace);
    window
        .update(cx, |_, window, _| window.remove_window())
        .expect("the window is open");
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            providers.read("workspace.kind", &plugin(), cx).err(),
            Some(HostError::Unavailable)
        );
    });
}
