//! Admission belongs to the installation across original root windows.
use super::*;

#[gpui_kit::test]
async fn admission_is_shared_between_separate_workspace_windows(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| settings.sessions.max_live = 1)
    })
    .await
    .unwrap();
    let first = draft(&f, cx);
    first.update(cx, |thread, cx| thread.send("window one".into(), cx));
    cx.run_until_parked();
    let (handle, _workspace, panel) = cx.update(|cx| {
        let mut panel = None;
        let (handle, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut workspace = Workspace::new(
                        crate::panel::tests::deps(f.installation.layout.clone()),
                        window,
                        cx,
                    );
                    let owner = cx.entity().downgrade();
                    let source = workspace.general_surface_source(cx);
                    let view = cx.new(|cx| {
                        AgentPanel::new(
                            f._directory
                                .host(owner, f.installation.installed.clone(), source),
                            window,
                            cx,
                        )
                    });
                    crate::panel::tests::host_panel(&mut workspace, &view, window, cx);
                    panel = Some(view);
                    workspace
                })
            })
            .unwrap();
        (handle, workspace, panel.unwrap())
    });
    let second = cx
        .update_window(handle, |_, window, cx| {
            panel.update(cx, |panel, cx| panel.new_thread("codex".into(), window, cx));
            panel.read(cx).current().unwrap()
        })
        .unwrap();
    second.update(cx, |thread, cx| thread.send("window two".into(), cx));
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 1);
    cx.update(|cx| assert_eq!(second.read(cx).composer().queue().len(), 1));
    complete_active(&f, cx);
    tick(cx, 1);
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 2);
    cx.update(|cx| {
        assert!(!first.read(cx).is_leased());
        assert!(second.read(cx).session_generating());
    });
    complete_active(&f, cx);
}
