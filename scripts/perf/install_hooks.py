#!/usr/bin/env python3
"""Install measurement hooks in an isolated benchmark checkout."""
import argparse
from pathlib import Path

STARTUP = '''        // PERF-HOOK (measurement only, never commit)
        if std::env::var_os("NOCTERM_PERF_AGENT").is_some() {
            workspace.update(cx, |w, cx| w.toggle_right_panel(window, cx));
        }
        if std::env::var_os("NOCTERM_PERF_LOCAL").is_some() {
            workspace.update(cx, |w, cx| w.toggle_local_terminal(window, cx));
        }
        if let Ok(target) = std::env::var("NOCTERM_PERF_SSH") {
            let count = std::env::var("NOCTERM_PERF_SSH_TABS")
                .ok().and_then(|n| n.parse::<usize>().ok()).unwrap_or(1);
            let key = std::env::var("NOCTERM_PERF_SSH_KEY").expect("SSH key path required");
            for _ in 0..count {
                let spec = nocterm_workspace::SessionSpec {
                    options: Default::default(), title: "Perf SSH".into(), profile: None,
                    target: nocterm_session::Target::parse(&target, None).unwrap(),
                    auth: nocterm_session::Auth::Key { path: key.clone().into() },
                    launch: None, credential: None,
                };
                workspace.update(cx, |w, cx| w.open_session(spec, window, cx));
            }
        }
        if std::env::var_os("NOCTERM_PERF_SSH").is_some() {
            let perf_workspace = workspace.clone();
            window.spawn(cx, async move |cx| {
                loop {
                    cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
                    let ready = cx.update(|_, cx| {
                        let terminals = perf_workspace.read(cx).terminals(cx);
                        let remote: Vec<_> = terminals.iter().filter_map(|t| t.access.info(cx))
                            .filter(|info| !info.local).collect();
                        !remote.is_empty() && remote.iter().all(|info|
                            info.status == nocterm_workspace::TerminalStatus::Connected)
                    }).unwrap_or(false);
                    if ready {
                        if let Some(run) = std::env::var_os("NOCTERM_PERF_RUN") {
                            let _ = std::fs::write(std::path::PathBuf::from(run).join("ssh-ready"), []);
                        }
                        break;
                    }
                }
            }).detach();
        }
        if let Some(run) = std::env::var_os("NOCTERM_PERF_RUN") {
            let _ = std::fs::write(std::path::PathBuf::from(run).join("app-ready"), []);
        }
        if let Ok(text) = std::env::var("NOCTERM_PERF_KEYS") {
            let perf_workspace = workspace.clone();
            window
                .spawn(cx, async move |cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(4))
                        .await;
                    let mut sent = 0usize;
                    let mut focused = 0usize;
                    let mut handled = 0usize;
                    for key in text.split(',').cycle() {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(66))
                            .await;
                        let _ = cx.update(|window, cx| {
                            let item = perf_workspace.read(cx).command_item(window, cx);
                            let target_ready = item.as_ref().is_some_and(|item| {
                                item.focus_handle(cx).contains_focused(window, cx)
                                    && item.terminal_access(cx).and_then(|t| t.info(cx))
                                        .is_some_and(|info| info.status == nocterm_workspace::TerminalStatus::Connected)
                            });
                            if !target_ready { return; }
                            focused += 1;
                            handled += usize::from(window.dispatch_keystroke(
                                gpui_kit::Keystroke::parse(key).unwrap(), cx));
                            sent += 1;
                            // One write per second, identical for both binaries.
                            if (sent == 1 || sent % 15 == 0)
                                && let Some(run) = std::env::var_os("NOCTERM_PERF_RUN")
                            {
                                let path = std::path::PathBuf::from(run);
                                let counters = format!("{sent} {focused} {handled}\\n");
                                let _ = std::fs::write(path.join("keys-count.tmp"), counters);
                                let _ = std::fs::rename(path.join("keys-count.tmp"), path.join("keys-count"));
                            }
                        });
                    }
                })
                .detach();
        }
'''
CHAT = '''        // PERF-HOOK (measurement only, never commit)
        if self.active.is_none() && std::env::var_os("NOCTERM_PERF_CHAT").is_some() {
            self.active = (0..self.threads.len())
                .max_by_key(|&ix| self.threads[ix].read(cx).state.entries.len());
        }
'''
DRAIN = '''                    // PERF-HOOK: acknowledge parsing, not merely producer completion.
                    if title.as_deref() == Some("NOCTERM-PERF-DRAINED")
                        && let Some(run) = std::env::var_os("NOCTERM_PERF_RUN")
                    {
                        let _ = std::fs::write(std::path::PathBuf::from(run).join("flood-drained"), []);
                    }
'''


def insert_once(path, marker, anchor, addition):
    source = path.read_text()
    if marker in source:
        # Repair the original marker hook on already instrumented scratch copies.
        repaired = source.replace('title == "NOCTERM-PERF-DRAINED"',
                                  'title.as_deref() == Some("NOCTERM-PERF-DRAINED")')
        if repaired != source:
            path.write_text(repaired)
        return
    if source.count(anchor) != 1:
        raise RuntimeError(f"expected exactly one hook anchor in {path}")
    path.write_text(source.replace(anchor, anchor + addition))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("checkout", type=Path)
    args = parser.parse_args()
    root = args.checkout.resolve()
    if (root / ".git").is_dir():
        parser.error("use an isolated archive or detached benchmark worktree")
    git = root / ".git"
    if git.is_file():
        import subprocess
        branch = subprocess.run(["git", "-C", str(root), "symbolic-ref", "--quiet", "HEAD"],
                                capture_output=True).stdout.decode().strip()
        if branch:
            parser.error("refusing to install hooks on a named development branch")
    main = root / "src/main.rs"
    source = main.read_text()
    # Upgrade only the exact benchmark-only block, preserving all product code.
    if 'NOCTERM_PERF_KEYS' in source:
        start = source.index('        // PERF-HOOK (measurement only, never commit)')
        end = source.index('        workspace\n', start)
        main.write_text(source[:start] + STARTUP + source[end:])
    else:
        insert_once(main, 'NOCTERM_PERF_KEYS',
                    '        window.focus(&workspace.focus_handle(cx), cx);\n', STARTUP)
    lifecycle = root / "crates/agent/src/panel/lifecycle.rs"
    panel = lifecycle if lifecycle.exists() else root / "crates/agent/src/panel.rs"
    insert_once(panel, 'NOCTERM_PERF_CHAT',
                '        self.active = self.active.map(|active| active + count);\n', CHAT)
    insert_once(root / "crates/terminal/src/terminal.rs", 'NOCTERM-PERF-DRAINED',
                '                Effect::Title(title) => {\n', DRAIN)


if __name__ == "__main__":
    main()
