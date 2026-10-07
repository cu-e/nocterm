//! Clipboard and Explorer drops share native terminal paste encoding.
use super::*;
use nocterm_workspace::FileDrag;

impl TerminalView {
    pub(super) fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.paste_text(&text, cx);
        }
    }

    fn paste_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let modes = self.terminal.read(cx).emulator().modes();
        self.type_text(
            std::str::from_utf8(&encode_paste(text, modes))
                .expect("paste wrapping preserves UTF-8"),
            cx,
        );
    }

    pub(super) fn drop_files(
        &mut self,
        files: &FileDrag,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        let terminal = self.terminal.read(cx);
        let result =
            if terminal.is_connected() && terminal.prompt().is_none() && !terminal.awaiting_vault()
            {
                dropped_paths(files).and_then(|paths| {
                    quote_paths_with_syntax(
                        &paths,
                        terminal.drop_shell(),
                        terminal.is_local(),
                        terminal.drop_syntax(),
                    )
                })
            } else {
                Err("Connect and finish authentication before dropping paths.".into())
            };
        match result {
            Ok(text) => {
                let result = self.terminal.update(cx, |terminal, cx| {
                    terminal.paste_paths(&text, cx)?;
                    terminal.update_emulator(cx, |emulator| {
                        emulator.scroll(Scroll::Bottom);
                        emulator.clear_selection();
                    });
                    Ok::<_, String>(())
                });
                if let Err(error) = result {
                    nocterm_ui::notice::warning(
                        window,
                        cx,
                        "terminal-file-drop",
                        "Could not insert file paths",
                        error,
                    );
                } else {
                    window.focus(&self.focus_handle, cx);
                    self.wake_cursor(cx);
                }
            }
            Err(error) => nocterm_ui::notice::warning(
                window,
                cx,
                "terminal-file-drop",
                "Could not insert file paths",
                error,
            ),
        }
    }
}

fn dropped_paths(files: &FileDrag) -> Result<Vec<String>, String> {
    match files {
        FileDrag::Local(paths) => paths
            .iter()
            .map(|path| {
                path.to_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "This path cannot be represented as terminal text.".into())
            })
            .collect(),
        FileDrag::Remote(files) => Ok(files.sources.clone()),
    }
}

#[cfg(test)]
fn quote_paths(paths: &[String], program: &str, local: bool) -> Result<String, String> {
    quote_paths_with_syntax(paths, program, local, None)
}

fn quote_paths_with_syntax(
    paths: &[String],
    program: &str,
    local: bool,
    syntax: Option<nocterm_workspace::ShellSyntax>,
) -> Result<String, String> {
    if paths.is_empty() {
        return Err("No file paths were selected.".into());
    }
    let executable = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let stem = executable.strip_suffix(".exe").unwrap_or(&executable);
    let powershell = match syntax {
        Some(nocterm_workspace::ShellSyntax::PowerShell) => true,
        Some(nocterm_workspace::ShellSyntax::Posix) => false,
        None => matches!(stem, "powershell" | "pwsh"),
    };
    let posix = matches!(stem, "bash" | "zsh" | "fish" | "sh" | "dash" | "ksh" | "ash" | "mksh" | "yash")
        // Remote unspecified programs use the server's login shell; local Unix
        // transport defaults to /bin/sh when SHELL is unavailable.
        || (stem.is_empty() && (!local || !cfg!(windows)));
    if !powershell && !posix && syntax != Some(nocterm_workspace::ShellSyntax::Posix) {
        return Err("Path insertion needs a supported POSIX shell or PowerShell.".into());
    }
    let mut length = 0usize;
    let mut quoted = Vec::with_capacity(paths.len());
    for path in paths {
        if path.chars().any(char::is_control) {
            return Err("Paths containing control characters cannot be inserted safely.".into());
        }
        let argument = if powershell {
            nocterm_session::quote_powershell(path)
        } else {
            nocterm_session::quote_posix(path)
        };
        length = length.saturating_add(argument.len() + 1);
        if length > 256 * 1024 {
            return Err("The selected paths exceed 256 KiB; select fewer items.".into());
        }
        quoted.push(argument);
    }
    Ok(quoted.join(" "))
}

#[cfg(test)]
#[path = "paste_tests.rs"]
mod native_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_literal_arguments_without_enter() {
        let paths = vec!["/a b/資料".into(), "/a'b/$(touch bad);*".into()];
        assert_eq!(
            quote_paths(&paths, "/bin/bash", true).unwrap(),
            "'/a b/資料' '/a'\\''b/$(touch bad);*'"
        );
        assert_eq!(
            quote_paths(&paths, "C:\\bin\\pwsh.exe", true).unwrap(),
            "'/a b/資料' '/a''b/$(touch bad);*'"
        );
    }

    #[test]
    fn executable_suffix_and_case_do_not_change_supported_quote_style() {
        assert_eq!(
            quote_paths(&["/a'b".into()], "C:\\bin\\PWSH.EXE", true).unwrap(),
            "'/a''b'"
        );
        assert_eq!(
            quote_paths(&["/a'b".into()], "/bin/BASH.EXE", false).unwrap(),
            "'/a'\\''b'"
        );
        assert!(quote_paths(&["/a'b".into()], "pwsh.exe.exe", false).is_err());
        assert!(quote_paths(&["/a'b".into()], "/bin/NU.EXE", false).is_err());
    }

    #[test]
    fn unsafe_or_excessive_input_is_rejected_atomically() {
        for control in ['\0', '\r', '\n', '\x1b', '\t', '\x7f'] {
            assert!(quote_paths(&[format!("/x{control}y")], "bash", true).is_err());
        }
        assert!(quote_paths(&[], "bash", true).is_err());
        assert!(quote_paths(&["/a".into()], "cmd.exe", true).is_err());
        assert!(quote_paths(&["x".repeat(256 * 1024)], "bash", true).is_err());
    }

    #[test]
    fn local_paths_preserve_order_for_any_receiving_terminal() {
        let files = FileDrag::Local(vec!["/home/a".into(), "/home/b".into()]);
        assert_eq!(dropped_paths(&files).unwrap(), ["/home/a", "/home/b"]);
    }
}
