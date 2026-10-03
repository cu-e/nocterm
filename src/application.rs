//! Process-wide actions share services, while each window owns its Workspace.

use std::path::PathBuf;

use gpui_kit::{
    App, Global,
    component::{WindowExt as _, button::Button, notification::Notification, v_flex},
    prelude::*,
};
use nocterm_core::Paths;
use nocterm_ui::ActiveSettings as _;
use nocterm_workspace::{About, CloseWindow, LogsDirectory, NewWindow, ProfilesDirectory, Quit};

// release-please's authoritative version file, also used by packaging.
pub(crate) const VERSION: &str = include_str!("../version.txt");

struct ApplicationState {
    paths: Paths,
    vault_ready: bool,
}
impl Global for ApplicationState {}

pub(crate) fn register(paths: Paths, vault_ready: bool, cx: &mut App) {
    cx.set_global(ApplicationState { paths, vault_ready });
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &NewWindow, cx| {
        let vault_ready = cx.global::<ApplicationState>().vault_ready;
        cx.defer(move |cx| {
            if let Err(error) = super::open_main_window(cx, vault_ready) {
                report_error(format!("Could not open a window: {error}"), cx);
            }
        });
    });
    cx.on_action(|_: &CloseWindow, cx| {
        if let Some(handle) = cx.active_window() {
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            });
        }
    });
    cx.on_action(|_: &About, cx| {
        if let Some(handle) = cx.active_window() {
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, cx| {
                    window.open_dialog(cx, |dialog, _, _| {
                        dialog
                            .title("About Nocterm")
                            .child(
                                v_flex()
                                    .gap_2()
                                    .child(format!("Nocterm {}", VERSION.trim()))
                                    .child("An extensible SSH client.")
                                    .child("PolyForm Perimeter License 1.0.1"),
                            )
                            .footer(
                                Button::new("about-close")
                                    .label("Close")
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            )
                    });
                });
            });
        }
    });
    cx.on_action(|_: &ProfilesDirectory, cx| {
        let directory = cx
            .global::<ApplicationState>()
            .paths
            .config_dir()
            .to_owned();
        open_directory(directory, cx);
    });
    cx.on_action(|_: &LogsDirectory, cx| {
        let directory = cx.settings().logging.directory.clone().unwrap_or_else(|| {
            cx.global::<ApplicationState>()
                .paths
                .state_dir()
                .join("logs")
        });
        open_directory(directory, cx);
    });
}

fn open_directory(directory: PathBuf, cx: &mut App) {
    let work = cx
        .background_executor()
        .spawn(async move { std::fs::create_dir_all(&directory).map(|()| directory) });
    cx.spawn(async move |cx| {
        let result = work.await;
        cx.update(|cx| match result {
            Ok(directory) => cx.open_with_system(&directory),
            Err(error) => report_error(format!("Could not open the directory: {error}"), cx),
        });
    })
    .detach();
}

fn report_error(message: String, cx: &mut App) {
    tracing::error!(%message);
    if let Some(handle) = cx.active_window() {
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                window.push_notification(Notification::error(message), cx);
            });
        });
    }
}
