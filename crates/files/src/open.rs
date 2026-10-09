//! Opening Explorer entries: files with the programs chosen in Settings →
//! Explorer, or the system's default application, and local entries in the
//! system file manager.

use gpui_kit::{
    App, SharedString, WeakEntity, Window,
    component::menu::{PopupMenu, PopupMenuItem},
};
use nocterm_session::fs::path;
use nocterm_settings::{OpenSettings, Opener};
use nocterm_ui::SettingsExt as _;
use nocterm_workspace::Workspace;
use std::{
    path::Path,
    process::{Command, Stdio},
};

use crate::operations::FileTarget;

/// The user's opener settings; the defaults where none are installed.
fn settings(cx: &App) -> OpenSettings {
    if cx.has_global::<nocterm_ui::SettingsStore>() {
        cx.setting::<nocterm_settings::ExplorerSettings>()
            .open
            .clone()
    } else {
        OpenSettings::default()
    }
}

/// The program that opens `target`; `None` means the system default.
fn opener(target: &FileTarget, cx: &App) -> Option<Opener> {
    let remote = matches!(target, FileTarget::Remote { .. });
    settings(cx).opener_for(&target.name(), remote).cloned()
}

/// Adds the ways to open `target` to the end of its context menu.
pub(crate) fn menu_items(
    menu: PopupMenu,
    target: &FileTarget,
    directory: bool,
    workspace: WeakEntity<Workspace>,
    cx: &App,
) -> PopupMenu {
    let mut menu = menu.separator();
    let local = match target {
        FileTarget::Local(path) => Some(path.clone()),
        FileTarget::Remote { .. } => None,
    };
    if !directory {
        let label: SharedString = match opener(target, cx) {
            Some(opener) => format!("Open with {}", opener.program).into(),
            None if local.is_some() => "Open".into(),
            None => "Open (choose a program in Settings → Explorer)".into(),
        };
        let open = target.clone();
        menu = menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
            let (open, workspace) = (open.clone(), workspace.clone());
            window.defer(cx, move |window, cx| run(&open, &workspace, window, cx));
        }));
        if let Some(path) = local.clone().filter(|_| opener(target, cx).is_some()) {
            menu = menu.item(
                PopupMenuItem::new("Open with default application")
                    .on_click(move |_, _, cx| cx.open_with_system(&path)),
            );
        }
    }
    if let Some(path) = local {
        menu = if directory {
            menu.item(
                PopupMenuItem::new("Open in file manager")
                    .on_click(move |_, _, cx| cx.open_with_system(&path)),
            )
        } else {
            menu.item(
                PopupMenuItem::new("Show in file manager")
                    .on_click(move |_, _, cx| cx.reveal_path(&path)),
            )
        };
    }
    menu
}

/// Opens a file once the current update is over, reporting failure as a
/// notice.
pub(crate) fn later(
    target: FileTarget,
    workspace: WeakEntity<Workspace>,
    window: gpui_kit::AnyWindowHandle,
    cx: &mut App,
) {
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, cx| run(&target, &workspace, window, cx));
    });
}

/// Opens a file, reporting failure as a notice.
pub(crate) fn run(
    target: &FileTarget,
    workspace: &WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    if let Err(error) = open(target, workspace, window, cx) {
        nocterm_ui::notice::error(
            window,
            cx,
            "files-open",
            "Could not open file",
            format!("{}: {error}", target.name()),
        );
    }
}

fn open(
    target: &FileTarget,
    workspace: &WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) -> Result<(), String> {
    let opener = opener(target, cx);
    match target {
        FileTarget::Local(file) => match opener {
            Some(opener) => {
                let text = file
                    .to_str()
                    .ok_or("This file name cannot be passed to a program.")?;
                spawn(&opener.program, opener.arguments(text), file.parent())
            }
            None => {
                cx.open_with_system(file);
                Ok(())
            }
        },
        FileTarget::Remote {
            path: file, host, ..
        } => {
            let opener = opener
                .ok_or("Choose the program that opens files on servers in Settings → Explorer.")?;
            let title = format!("{} {}", opener.program, path::file_name(file));
            let workspace = workspace.upgrade().ok_or("The window was closed.")?;
            workspace.update(cx, |workspace, cx| {
                workspace.open_on_host(
                    host,
                    opener.program.clone(),
                    opener.arguments(file),
                    Some(path::parent(file).to_owned()),
                    title.into(),
                    window,
                    cx,
                )
            })
        }
    }
}

/// Starts a program detached from Nocterm's terminal; a thread reaps it.
fn spawn(program: &str, args: Vec<String>, directory: Option<&Path>) -> Result<(), String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not start {program}: {error}"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn local_programs_get_the_file_and_run_in_its_folder() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("opened");
        let script = format!("printf '%s' \"$1\" > {}", marker.display());
        spawn(
            "/bin/sh",
            vec!["-c".into(), script, "sh".into(), "a b.txt".into()],
            Some(directory.path()),
        )
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::fs::read_to_string(&marker).unwrap_or_default() != "a b.txt" {
            assert!(std::time::Instant::now() < deadline, "program did not run");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let error = spawn("/nonexistent/editor", Vec::new(), None).unwrap_err();
        assert!(error.contains("/nonexistent/editor"), "{error}");
    }
}
