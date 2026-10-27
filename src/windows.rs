//! Windows of the application: each shows one environment, chosen when it is composed.
//!
//! A window is this computer or one server of nocterm hubs. Opening a window and switching the
//! environment of an open one compose the same workspaces; only the second keeps the native
//! window, so its place and size stay. The environments of the open windows are reported to the
//! window set, which the next start restores from ([`restore_windows`]).

mod restore;

use std::rc::Rc;

use gpui_kit::{
    App, AppContext as _, Bounds, Entity, Focusable as _, Window, WindowBounds, WindowOptions,
    component::TitleBar, px, size,
};
use nocterm_core::error::SharedCause;
use nocterm_hubs_model::Environment;
use nocterm_hubs_ui::{EnvironmentOpenError, EnvironmentOpener, Placement};
use nocterm_workspace::Workspace;

pub(crate) use restore::restore_windows;

use crate::{APP_ID, bootstrap::Booted, shell_setup::hub_window};

/// Opens a window showing `environment`.
pub(crate) fn open_window(
    cx: &mut App,
    booted: &Booted,
    environment: Environment,
) -> Result<(), EnvironmentOpenError> {
    let title = title_of(booted, &environment, cx)?;
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

    let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
        window.set_window_title(&title);
        // "System" appearance follows the desktop as it changes.
        let themes = booted.themes().clone();
        window
            .observe_window_appearance(move |_, cx| themes.apply(cx))
            .detach();
        let workspace = compose(booted, &environment, window, cx);
        window.focus(&workspace.focus_handle(cx), cx);
        workspace
    })
    .map_err(|error| {
        EnvironmentOpenError::Window(SharedCause::new(std::io::Error::other(error)))
    })?;
    booted
        .windowing()
        .opened(window.window_id(), environment, cx);
    Ok(())
}

/// Replaces what `window` shows with `environment`, keeping the native window. The previous
/// workspace is dropped: local sessions are closed and hub tabs detach.
pub(crate) fn replace_environment(
    window: &mut Window,
    booted: &Booted,
    environment: &Environment,
    cx: &mut App,
) -> Result<(), EnvironmentOpenError> {
    let title = title_of(booted, environment, cx)?;
    let mut built = None;
    window.replace_root(cx, |window, cx| {
        let workspace = compose(booted, environment, window, cx);
        built = Some(workspace.clone());
        gpui_kit::base::Root::new(workspace, window, cx)
    });
    window.set_window_title(&title);
    if let Some(workspace) = built {
        window.focus(&workspace.focus_handle(cx), cx);
    }
    booted
        .windowing()
        .switched(window.window_handle().window_id(), environment.clone(), cx);
    Ok(())
}

/// The title of a window showing `environment`; refuses a server that is not in the list.
fn title_of(
    booted: &Booted,
    environment: &Environment,
    cx: &App,
) -> Result<String, EnvironmentOpenError> {
    match environment {
        Environment::Local => Ok("Nocterm".to_owned()),
        Environment::Hub(hub) => booted
            .hubs()
            .and_then(|hubs| hubs.label(hub, cx))
            .map(|label| format!("Nocterm \u{2014} {label}"))
            .ok_or_else(|| EnvironmentOpenError::UnknownHub(hub.clone())),
    }
}

/// Makes the workspace of `environment` for `window`.
pub(crate) fn compose(
    booted: &Booted,
    environment: &Environment,
    window: &mut Window,
    cx: &mut App,
) -> Entity<Workspace> {
    cx.new(|cx| {
        let mut workspace = Workspace::new(booted.workspace_deps(), window, cx);
        match environment {
            Environment::Local => {
                crate::bootstrap::build_workspace(&mut workspace, booted, window, cx)
            }
            Environment::Hub(hub) => {
                hub_window::build_hub_workspace(&mut workspace, booted, hub, window, cx);
            }
        }
        workspace
    })
}

/// Opens the environments the menus and the sign-in dialog ask for.
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
        title_of(&self.booted, &environment, cx)?;
        let booted = self.booted.clone();
        match placement {
            Placement::NewWindow => cx.defer(move |cx| {
                if let Err(error) = open_window(cx, &booted, environment) {
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
                        if let Err(error) = replace_environment(window, &booted, &environment, cx) {
                            crate::application::report_error(
                                format!("Could not switch the window: {error}"),
                                cx,
                            );
                        }
                    });
                });
            }
        }
        Ok(())
    }
}
