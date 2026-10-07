//! The actual panel shell operation supplies syntax; log tabs do not.
use super::*;
use crate::tests::{FakeHost, fixture};
use gpui_kit::TestAppContext;
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
