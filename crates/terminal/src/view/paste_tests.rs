//! Native drop routing exercises the shared payload and the receiving terminal.
use super::super::tests::{drain, emit, fixture, scripted_driver};
use super::*;
use gpui_kit::{TestAppContext, base::TestSupportExt as _, px, test::TestWindowExt as _};
use nocterm_session::Event;

struct DropHarness {
    view: Entity<TerminalView>,
    files: FileDrag,
}
struct Preview;
impl Render for Preview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child("File")
    }
}
impl Render for DropHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .size_full()
            .child(
                div()
                    .id("test-file-source")
                    .test_support()
                    .w(px(100.))
                    .h(px(48.))
                    .child("File paths")
                    .on_drag(self.files.clone(), |_, _, _, cx| cx.new(|_| Preview)),
            )
            .child(div().flex_1().min_w_0().h_full().child(self.view.clone()))
    }
}

#[gpui_kit::test]
fn native_file_drop_inserts_literal_paths_once_without_enter(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    emit(cx, &transport, 0, Event::Output(b"\x1b[?2004h".to_vec()));
    let driver = scripted_driver(&transport, 0);
    drain(&driver);
    cx.update_window(handle, |_, window, cx| {
        window.replace_root(cx, |_, _| DropHarness {
            view: view.clone(),
            files: FileDrag::Local(vec!["/tmp/a b".into(), "/tmp/資料's".into()]),
        });
        window.render_frame(cx);
        window.drag_to("test-file-source", "terminal-drop-target", cx);
    })
    .unwrap();
    assert_eq!(
        drain(&driver),
        vec![
            "\x1b[200~'/tmp/a b' '/tmp/資料'\\''s'\x1b[201~"
                .as_bytes()
                .to_vec()
        ]
    );
}

#[gpui_kit::test]
fn remote_shell_launch_override_controls_quote_style_for_local_source(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    let view = cx
        .update_window(handle, |_, window, cx| {
            let mut spec = view.read(cx).terminal.read(cx).spec().clone();
            spec.launch = Some(nocterm_session::ShellLaunch {
                program: Some("/usr/bin/pwsh".into()),
                ..Default::default()
            });
            cx.new(|cx| TerminalView::new(spec, window, cx))
        })
        .unwrap();
    emit(cx, &transport, 1, Event::Connected);
    let driver = scripted_driver(&transport, 1);
    drain(&driver);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.drop_files(&FileDrag::Local(vec!["/a'b".into()]), window, cx)
        });
    })
    .unwrap();
    assert_eq!(drain(&driver), vec![b"'/a''b'".to_vec()]);
}

#[gpui_kit::test]
fn native_remote_source_drop_uses_paths_and_focuses_receiving_terminal(cx: &mut TestAppContext) {
    let (handle, view, transport) = fixture(cx);
    let driver = scripted_driver(&transport, 0);
    drain(&driver);
    let files = FileDrag::Remote(nocterm_workspace::RemoteFileDrag {
        sources: vec!["/remote/a b".into(), "/remote/資料's".into()],
        target: nocterm_session::Target::new("other-user", "other-host", 22),
        fs: view.read_with(cx, |view, cx| {
            view.terminal.read(cx).session_context().fs.unwrap()
        }),
    });
    cx.update_window(handle, |_, window, cx| {
        window.replace_root(cx, |_, _| DropHarness {
            view: view.clone(),
            files,
        });
        window.render_frame(cx);
        window.drag_to("test-file-source", "terminal-drop-target", cx);
        assert!(view.read(cx).focus_handle.is_focused(window));
    })
    .unwrap();
    assert_eq!(
        drain(&driver),
        vec!["'/remote/a b' '/remote/資料'\\''s'".as_bytes().to_vec()]
    );
}

#[cfg(unix)]
#[gpui_kit::test]
fn native_non_utf8_path_rejects_whole_drop_with_notice(cx: &mut TestAppContext) {
    use std::os::unix::ffi::OsStringExt as _;
    let (handle, view, transport) = fixture(cx);
    let driver = scripted_driver(&transport, 0);
    drain(&driver);
    let invalid =
        std::path::PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/file-\xff".to_vec()));
    cx.update_window(handle, |_, window, cx| {
        window.replace_root(cx, |_, _| DropHarness {
            view: view.clone(),
            files: FileDrag::Local(vec!["/tmp/valid-first".into(), invalid]),
        });
        window.render_frame(cx);
        window.drag_to("test-file-source", "terminal-drop-target", cx);
        assert_eq!(nocterm_ui::notice::count(window, cx), 1);
    })
    .unwrap();
    assert!(
        drain(&driver).is_empty(),
        "A bad member must reject the complete drop"
    );
}

#[test]
fn explicitly_unsupported_shells_do_not_receive_posix_quoted_paths() {
    for shell in ["/usr/bin/nu", "/usr/bin/xonsh", "custom-shell"] {
        assert!(
            quote_paths(&["/a'b".into()], shell, false).is_err(),
            "Unsupported shell: {shell}"
        );
    }
    for shell in [
        "",
        "/bin/bash",
        "/bin/zsh",
        "/usr/bin/fish",
        "/bin/sh",
        "/bin/dash",
        "/bin/ksh",
        "/bin/ash",
        "/usr/bin/pwsh",
    ] {
        assert!(
            quote_paths(&["/a'b".into()], shell, false).is_ok(),
            "Supported or default shell: {shell}"
        );
    }
}

#[derive(Default)]
struct ProgramHost {
    drivers: std::sync::Mutex<Vec<std::sync::Arc<nocterm_session::SessionDriver>>>,
    requests: std::sync::Mutex<Vec<nocterm_session::TerminalRequest>>,
}
impl nocterm_session::HostExec for ProgramHost {
    fn exec(
        &self,
        _: nocterm_session::ExecRequest,
    ) -> nocterm_session::ExecFuture<nocterm_session::ExecOutput> {
        Box::pin(async { Err(nocterm_session::ExecError::Unsupported) })
    }
    fn terminal(&self, request: nocterm_session::TerminalRequest) -> nocterm_session::Session {
        let (session, driver) = nocterm_session::channel(None);
        self.requests.lock().unwrap().push(request);
        self.drivers
            .lock()
            .unwrap()
            .push(std::sync::Arc::new(driver));
        session
    }
}
fn program_event(
    driver: &std::sync::Arc<nocterm_session::SessionDriver>,
    event: Event,
    cx: &mut TestAppContext,
) {
    let driver = driver.clone();
    cx.background_executor
        .spawn(async move {
            let _ = driver.emit(event).await;
        })
        .detach();
    cx.run_until_parked();
}
fn program_fixture(
    engine: &str,
    remote: bool,
    shell_syntax: Option<nocterm_workspace::ShellSyntax>,
    cx: &mut TestAppContext,
) -> (
    gpui_kit::AnyWindowHandle,
    Entity<TerminalView>,
    std::sync::Arc<nocterm_session::SessionDriver>,
) {
    use nocterm_workspace::{Host, HostKey, ProgramSpec, SessionContext};
    let (handle, _, _) = fixture(cx);
    let host = std::sync::Arc::new(ProgramHost::default());
    let target = nocterm_session::Target::new("user", "host", 22);
    let program =
        nocterm_session::ExecRequest::new(engine).args(["exec", "-it", "container", "sh"]);
    let view = cx
        .update_window(handle, |_, window, cx| {
            let terminal = cx.new(|cx| {
                crate::Terminal::new_program(
                    ProgramSpec {
                        title: "Container shell".into(),
                        host: Host {
                            key: if remote {
                                HostKey::Remote(target.clone())
                            } else {
                                HostKey::Local
                            },
                            exec: host.clone(),
                            connected: true,
                            session: remote.then(|| SessionContext::new(target, None, true)),
                        },
                        program: program.clone(),
                        shell_syntax,
                    },
                    cx,
                )
            });
            cx.new(|cx| TerminalView::with_terminal(terminal, window, cx))
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        host.requests.lock().unwrap()[0].program,
        program,
        "Preserve actual wrapper launch"
    );
    let driver = host.drivers.lock().unwrap()[0].clone();
    program_event(&driver, Event::Connected, cx);
    drain(&driver);
    cx.update_window(handle, |_, window, cx| {
        window.replace_root(cx, |_, _| DropHarness {
            view: view.clone(),
            files: FileDrag::Local(vec!["/tmp/a'b 資料".into()]),
        });
        window.render_frame(cx);
    })
    .unwrap();
    (handle, view, driver)
}

#[gpui_kit::test]
fn native_container_wrapper_uses_explicit_receiving_syntax_and_keeps_state_guards(
    cx: &mut TestAppContext,
) {
    for engine in ["docker", "podman"] {
        for remote in [false, true] {
            let (handle, view, driver) = program_fixture(
                engine,
                remote,
                Some(nocterm_workspace::ShellSyntax::Posix),
                cx,
            );
            cx.update_window(handle, |_, window, cx| {
                window.drag_to("test-file-source", "terminal-drop-target", cx);
            })
            .unwrap();
            assert_eq!(
                drain(&driver),
                vec!["'/tmp/a'\\''b 資料'".as_bytes().to_vec()]
            );
            let (reply, _answer) = nocterm_session::Reply::channel();
            program_event(
                &driver,
                Event::Prompt(nocterm_session::Prompt::Secret {
                    request: nocterm_session::SecretRequest::Interactive {
                        prompt: "challenge".into(),
                        echo: false,
                    },
                    reply,
                }),
                cx,
            );
            cx.update_window(handle, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.drop_files(&FileDrag::Local(vec!["/blocked".into()]), window, cx)
                });
            })
            .unwrap();
            assert!(
                drain(&driver).is_empty(),
                "Syntax metadata must not bypass authentication"
            );
            program_event(
                &driver,
                Event::Closed(nocterm_session::CloseReason::Exited(Some(0))),
                cx,
            );
            cx.update_window(handle, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.drop_files(&FileDrag::Local(vec!["/blocked".into()]), window, cx)
                });
            })
            .unwrap();
            assert!(
                drain(&driver).is_empty(),
                "Syntax metadata must not bypass closed state"
            );
        }
    }
}

#[gpui_kit::test]
fn native_program_wrapper_without_shell_metadata_refuses_path_drop(cx: &mut TestAppContext) {
    let (handle, _, driver) = program_fixture("docker", true, None, cx);
    cx.update_window(handle, |_, window, cx| {
        window.drag_to("test-file-source", "terminal-drop-target", cx);
        assert_eq!(nocterm_ui::notice::count(window, cx), 1);
    })
    .unwrap();
    assert!(
        drain(&driver).is_empty(),
        "Program and log tabs need explicit shell metadata"
    );
}
