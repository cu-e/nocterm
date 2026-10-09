//! The actual panel shell operation supplies syntax; log tabs do not.
use super::*;
use crate::tests::{FakeHost, fixture};
use gpui_kit::{TestAppContext, test::TestWindowExt as _};
use std::sync::{Arc, Mutex};

#[gpui_kit::test]
fn shell_operation_marks_posix_inner_shell_and_logs_leave_syntax_unset(cx: &mut TestAppContext) {
    let fixture = fixture(cx, Some(FakeHost::new(true)));
    let opened = Arc::new(Mutex::new(Vec::new()));
    let observed = opened.clone();
    fixture.workspace.update(cx, |workspace, _| {
        workspace.set_program_opener(move |_, spec, _, _| {
            observed.lock().unwrap().push(spec);
        });
    });
    for op in [Op::Shell, Op::Logs] {
        cx.update_window(fixture.window, |_, window, cx| {
            fixture.panel.update(cx, |panel, cx| {
                panel.run(
                    op,
                    Subject {
                        kind: Kind::Container,
                        name: "web".into(),
                        ids: vec!["a1".into()],
                    },
                    window,
                    cx,
                )
            });
        })
        .unwrap();
    }
    let specs = opened.lock().unwrap();
    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0].shell_syntax, Some(ShellSyntax::Posix));
    assert_eq!(
        specs[0].program,
        nocterm_containers::Engine::Docker.shell("a1")
    );
    assert_eq!(specs[1].shell_syntax, None);
    assert_eq!(
        specs[1].program,
        nocterm_containers::Engine::Docker.logs("a1")
    );
}

#[gpui_kit::test]
fn delete_confirmation_cannot_apply_on_a_new_host(cx: &mut TestAppContext) {
    let local = FakeHost::new(true);
    let fixture = fixture(cx, Some(local.clone()));
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.panel.update(cx, |panel, cx| {
            panel.run(
                Op::Act(Action::RemoveImage),
                Subject {
                    kind: Kind::Image,
                    name: "shared-image".into(),
                    ids: vec!["shared-image".into()],
                },
                window,
                cx,
            );
        });
        assert!(window.has_active_dialog(cx));
    })
    .unwrap();
    let remote = FakeHost::new(true);
    crate::tests::open_session(cx, &fixture, crate::tests::remote(&remote, true));
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("ok", cx);
    })
    .unwrap();
    cx.run_until_parked();
    for host in [&local, &remote] {
        assert!(
            !host
                .ran()
                .iter()
                .any(|line| line == "docker rmi shared-image")
        );
    }
}
