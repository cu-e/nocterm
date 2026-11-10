//! Windows of the application: each shows one environment, chosen when it is composed.
//!
//! A window is this computer or one hub account. Composing a window gives its workspace the
//! features of the environment's sources and puts the focus in it, so that actions of the
//! window (such as unlocking its vault) reach it.

use std::rc::Rc;

use gpui_kit::{
    App, AppContext as _, Bounds, Entity, Focusable as _, Window, WindowBounds, WindowOptions,
    component::TitleBar, px, size,
};
use nocterm_core::error::SharedCause;
use nocterm_hubs_model::Environment;
use nocterm_hubs_ui::{EnvironmentOpenError, EnvironmentOpener, Placement};
use nocterm_workspace::Workspace;

use crate::APP_ID;
use crate::bootstrap::{Booted, build_workspace};
use crate::window_sources::{WindowSources, WindowSourcesError};

/// Opens a window showing `environment`.
pub(crate) fn open_window(
    cx: &mut App,
    booted: &Booted,
    environment: &Environment,
) -> Result<(), EnvironmentOpenError> {
    let (sources, title) = sources_and_title(booted, environment, cx)?;
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1200.), px(760.)),
            cx,
        ))),
        window_min_size: Some(size(px(640.), px(400.))),
        app_id: Some(APP_ID.to_owned()),
        ..TitleBar::window_options()
    };
    gpui_kit::open_window(options, cx, |window, cx| {
        window.set_window_title(&title);
        // "System" appearance follows the desktop as it changes.
        let themes = booted.themes().clone();
        window
            .observe_window_appearance(move |_, cx| themes.apply(cx))
            .detach();
        compose(booted, &sources, window, cx)
    })
    .map_err(|error| {
        EnvironmentOpenError::Window(SharedCause::new(std::io::Error::other(error)))
    })?;
    Ok(())
}

/// Replaces what `window` shows with `sources`, keeping the native window. The previous
/// workspace is dropped: local sessions are closed.
fn replace_environment(
    window: &mut Window,
    booted: &Booted,
    sources: &WindowSources,
    title: &str,
    cx: &mut App,
) {
    let mut built = None;
    window.replace_root(cx, |window, cx| {
        let workspace = compose(booted, sources, window, cx);
        built = Some(workspace.clone());
        gpui_kit::base::Root::new(workspace, window, cx)
    });
    window.set_window_title(title);
    // The new root is in place only now: focus it so that its window actions are reachable.
    if let Some(workspace) = built {
        window.focus(&workspace.focus_handle(cx), cx);
    }
}

/// The sources and the title of a window showing `environment`; refuses a server that is not in
/// the list or whose account cannot be opened.
fn sources_and_title(
    booted: &Booted,
    environment: &Environment,
    cx: &mut App,
) -> Result<(WindowSources, String), EnvironmentOpenError> {
    let title = match environment {
        Environment::Local => "Nocterm".to_owned(),
        Environment::Hub(hub) => booted
            .hubs()
            .and_then(|hubs| hubs.label(hub, cx))
            .map(|label| format!("Nocterm \u{2014} {label}"))
            .ok_or_else(|| EnvironmentOpenError::UnknownHub(hub.clone()))?,
    };
    let sources =
        booted
            .window_sources(environment, cx)
            .map_err(|error| match (error, environment) {
                (WindowSourcesError::Account(error), _) => EnvironmentOpenError::Account(error),
                (WindowSourcesError::NoHubs, Environment::Hub(hub)) => {
                    EnvironmentOpenError::UnknownHub(hub.clone())
                }
                (error, _) => EnvironmentOpenError::Window(SharedCause::new(error)),
            })?;
    Ok((sources, title))
}

/// Opens the environments the menu and the sign-in dialog ask for.
pub(crate) struct AppEnvironmentOpener {
    booted: Booted,
}

impl AppEnvironmentOpener {
    pub(crate) fn new(booted: Booted) -> Rc<Self> {
        Rc::new(Self { booted })
    }
}

impl EnvironmentOpener for AppEnvironmentOpener {
    fn open(
        &self,
        environment: Environment,
        placement: Placement,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), EnvironmentOpenError> {
        // Refuse here, where the caller can still tell the user.
        let (sources, title) = sources_and_title(&self.booted, &environment, cx)?;
        let booted = self.booted.clone();
        match placement {
            Placement::NewWindow => cx.defer(move |cx| {
                if let Err(error) = open_window(cx, &booted, &environment) {
                    crate::application::report_error(
                        format!("Could not open a window: {error}"),
                        cx,
                    );
                }
            }),
            Placement::ThisWindow => {
                // The workspace asking is dropped by the replacement, so it happens after the
                // caller returns.
                let handle = window.window_handle();
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        replace_environment(window, &booted, &sources, &title, cx);
                    });
                });
            }
        }
        Ok(())
    }
}

/// Makes the workspace of a window from `sources` and focuses it.
///
/// A hub window whose account's vault is locked asks for it at once: nothing of the account is
/// readable before.
pub(crate) fn compose(
    booted: &Booted,
    sources: &WindowSources,
    window: &mut Window,
    cx: &mut App,
) -> Entity<Workspace> {
    let workspace = cx.new(|cx| {
        let mut workspace = Workspace::new(booted.workspace_deps(), window, cx);
        build_workspace(&mut workspace, booted, sources, window, cx);
        workspace
    });
    window.focus(&workspace.focus_handle(cx), cx);
    if !sources.environment().is_local()
        && sources
            .vault()
            .is_some_and(|vault| vault.service().status() == nocterm_vault::VaultStatus::Locked)
    {
        // The actions of the window are reachable once its workspace is focused and has been
        // drawn, which is the frame after the next.
        window.on_next_frame(|window, _cx| {
            window.on_next_frame(|window, cx| {
                window.dispatch_action(Box::new(nocterm_workspace::UnlockVault), cx);
            });
        });
    }
    workspace
}
