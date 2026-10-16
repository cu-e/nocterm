//! Opening Explorer entries: files with the programs chosen in Settings →
//! Explorer, or the system's default application, and local entries in the
//! system file manager.

use crate::Opener;
use gpui_kit::{
    App, SharedString, WeakEntity, Window,
    component::menu::{PopupMenu, PopupMenuItem},
};
use nocterm_session::fs::path;
use nocterm_session::{ProgramOpenError, ProgramOpenRequest, ProgramOpener};
use nocterm_workspace::Workspace;
use std::sync::Arc;

use crate::operations::FileTarget;

/// The program that opens `target`; `None` means the system default.
fn opener(target: &FileTarget, installed: &crate::Installed, cx: &App) -> Option<Opener> {
    let remote = matches!(target, FileTarget::Remote { .. });
    installed
        .settings()
        .setting::<crate::ExplorerSettings>(cx)
        .open
        .opener_for(&target.name(), remote)
        .cloned()
}

/// Adds the ways to open `target` to the end of its context menu.
pub(crate) fn menu_items(
    menu: PopupMenu,
    target: &FileTarget,
    directory: bool,
    workspace: WeakEntity<Workspace>,
    installed: crate::Installed,
    cx: &App,
) -> PopupMenu {
    let mut menu = menu.separator();
    let local = match target {
        FileTarget::Local { path, .. } => Some(path.clone()),
        FileTarget::Remote { .. } => None,
    };
    if !directory {
        let label: SharedString = match opener(target, &installed, cx) {
            Some(opener) => format!("Open with {}", opener.program).into(),
            None if local.is_some() => "Open".into(),
            None => "Open (choose a program in Settings → Explorer)".into(),
        };
        let open = target.clone();
        let captured = installed.clone();
        menu = menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
            let (open, workspace, installed) = (open.clone(), workspace.clone(), captured.clone());
            window.defer(cx, move |window, cx| {
                run(&open, &workspace, &installed, window, cx)
            });
        }));
        if let Some(path) = local
            .clone()
            .filter(|_| opener(target, &installed, cx).is_some())
        {
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
    installed: crate::Installed,
    window: gpui_kit::AnyWindowHandle,
    cx: &mut App,
) {
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, cx| {
            run(&target, &workspace, &installed, window, cx)
        });
    });
}

/// Dispatches presentation intent through the supplied native or remote port.
fn run(
    target: &FileTarget,
    workspace: &WeakEntity<Workspace>,
    installed: &crate::Installed,
    window: &mut Window,
    cx: &mut App,
) {
    match target {
        FileTarget::Local { path: file, .. } => {
            match local_request(file, opener(target, installed, cx).as_ref()) {
                Ok(Some(request)) => launch(
                    installed.opener().clone(),
                    request,
                    target.name(),
                    window,
                    cx,
                ),
                Ok(None) => cx.open_with_system(file),
                Err(error) => report(target.name(), error, window, cx),
            }
        }
        FileTarget::Remote {
            path: file, host, ..
        } => {
            open_remote(target, file, host, workspace, installed, window, cx);
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum OpenError {
    #[error("This file name cannot be passed to a program.")]
    NativeFileName,
    #[error("Choose the program that opens files on servers in Settings → Explorer.")]
    MissingRemoteProgram,
    #[error("The window was closed.")]
    ClosedWindow,
    #[error(transparent)]
    Program(#[from] ProgramOpenError),
}

fn local_request(
    file: &std::path::Path,
    opener: Option<&Opener>,
) -> Result<Option<ProgramOpenRequest>, OpenError> {
    let Some(opener) = opener else {
        return Ok(None);
    };
    let text = file.to_str().ok_or(OpenError::NativeFileName)?;
    Ok(Some(ProgramOpenRequest::new(
        opener.program.clone(),
        opener.arguments(text),
        file.parent().map(ToOwned::to_owned),
    )?))
}

fn launch(
    native: Arc<dyn ProgramOpener>,
    request: ProgramOpenRequest,
    name: String,
    window: &mut Window,
    cx: &mut App,
) {
    let window = window.window_handle();
    cx.spawn(async move |cx| {
        if let Err(error) = native.open(request).await {
            let _ = window.update(cx, |_, window, cx| report(name, error, window, cx));
        }
    })
    .detach();
}

fn open_remote(
    target: &FileTarget,
    file: &str,
    host: &nocterm_session::Target,
    workspace: &WeakEntity<Workspace>,
    installed: &crate::Installed,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(opener) = opener(target, installed, cx) else {
        report(target.name(), OpenError::MissingRemoteProgram, window, cx);
        return;
    };
    let Some(workspace) = workspace.upgrade() else {
        report(target.name(), OpenError::ClosedWindow, window, cx);
        return;
    };
    let title = format!("{} {}", opener.program, path::file_name(file));
    // Format the typed workspace refusal only at the display surface.
    let result = workspace.update(cx, |workspace, cx| {
        workspace.open_on_host(
            host,
            opener.program.clone(),
            opener.arguments(file),
            Some(path::parent(file).to_owned()),
            title.into(),
            window,
            cx,
        )
    });
    if let Err(error) = result {
        report(target.name(), error, window, cx);
    }
}

fn report(name: String, error: impl std::fmt::Display, window: &mut Window, cx: &mut App) {
    nocterm_ui::notice::error(
        window,
        cx,
        "files-open",
        "Could not open file",
        format!("{name}: {error}"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn system_default_does_not_construct_an_external_program_request() {
        assert!(
            local_request(std::path::Path::new("note.txt"), None)
                .unwrap()
                .is_none()
        );
    }
    #[cfg(unix)]
    #[test]
    fn system_default_preserves_a_native_file_name_unsupported_by_configured_programs() {
        use std::os::unix::ffi::OsStrExt as _;
        let file = std::path::Path::new(std::ffi::OsStr::from_bytes(b"note-\xff.txt"));
        assert!(local_request(file, None).unwrap().is_none());
        assert!(matches!(
            local_request(file, Some(&Opener::new("editor", &[]))),
            Err(OpenError::NativeFileName)
        ));
    }
}
