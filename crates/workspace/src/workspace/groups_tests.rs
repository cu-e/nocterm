//! Groups of tabs in a live dock: kept together through moves, marked with
//! their color, ended by the tab menu, and formed for programs opened from
//! a session.
use std::sync::Arc;

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, Entity, EntityId, EventEmitter, FocusHandle,
    Focusable, TestAppContext, Window, component::Placement, div, prelude::*,
};
use nocterm_session::{ExecError, ExecFuture, ExecOutput, ExecRequest, HostExec, Target};

use crate::workspace::tests::fixture;
use crate::{Host, HostKey, Item, ItemEvent, ProgramSpec, SessionContext, Workspace};

struct Tab {
    focus: FocusHandle,
    session: Option<SessionContext>,
}
impl EventEmitter<ItemEvent> for Tab {}
impl Focusable for Tab {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for Tab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().track_focus(&self.focus)
    }
}
impl Item for Tab {
    fn tab_title(&self, _: &App) -> gpui_kit::SharedString {
        "Tab".into()
    }
    fn session(&self, _: &App) -> Option<SessionContext> {
        self.session.clone()
    }
}

struct NoExec;
impl HostExec for NoExec {
    fn exec(&self, _: ExecRequest) -> ExecFuture<ExecOutput> {
        Box::pin(async { Err(ExecError::Unsupported) })
    }
}

fn tab(session: Option<SessionContext>, cx: &mut App) -> Entity<Tab> {
    cx.new(|cx| Tab {
        focus: cx.focus_handle(),
        session,
    })
}

/// Opens `count` tabs in one pane, left to right.
fn open(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    count: usize,
) -> Vec<EntityId> {
    let ids = cx
        .update_window(window, |_, window, cx| {
            (0..count)
                .map(|_| {
                    let tab = tab(None, cx);
                    workspace.update(cx, |workspace, cx| {
                        workspace.add_item(tab.clone(), window, cx)
                    });
                    tab.entity_id()
                })
                .collect()
        })
        .unwrap();
    cx.run_until_parked();
    ids
}

/// Runs `f` on the workspace, then lets the dock settle.
fn with(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    f: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>),
) {
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| f(workspace, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

/// The items in each tab bar, left to right.
fn bars(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Vec<Vec<EntityId>> {
    workspace.read_with(cx, |workspace, cx| {
        workspace
            .tab_bars(cx)
            .into_iter()
            .map(|(_, panels)| {
                panels
                    .iter()
                    .filter_map(|panel| {
                        workspace
                            .items
                            .iter()
                            .find(|open| super::panel(open) == *panel)
                            .map(|open| open.handle.item_id())
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|bar| !bar.is_empty())
            .collect()
    })
}

fn color(cx: &mut TestAppContext, workspace: &Entity<Workspace>, item: EntityId) -> Option<usize> {
    workspace.read_with(cx, |workspace, cx| {
        let open = workspace
            .items
            .iter()
            .find(|open| open.handle.item_id() == item)?;
        open.dock_item.read(cx).group_color()
    })
}

#[gpui_kit::test]
fn a_group_sits_together_moves_as_one_and_is_marked(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    let [a, b, c, d] = open(cx, window, &workspace, 4)[..] else {
        unreachable!()
    };

    with(cx, window, &workspace, |ws, window, cx| {
        ws.group_tab_with(d, a, window, cx)
    });
    assert_eq!(
        bars(cx, &workspace),
        [vec![a, d, b, c]],
        "the tab joins its group"
    );
    assert!(color(cx, &workspace, a).is_some());
    assert_eq!(color(cx, &workspace, a), color(cx, &workspace, d));
    assert_eq!(color(cx, &workspace, b), None);

    // `a` dragged one step right, past its own group, then once more.
    with(cx, window, &workspace, |ws, window, cx| {
        ws.activate_item_by_id(a, window, cx);
        ws.move_active_tab(1, window, cx);
    });
    assert_eq!(bars(cx, &workspace), [vec![d, a, b, c]], "reordered inside");
    with(cx, window, &workspace, |ws, window, cx| {
        ws.move_active_tab(1, window, cx)
    });
    assert_eq!(
        bars(cx, &workspace),
        [vec![b, d, a, c]],
        "the group follows"
    );

    with(cx, window, &workspace, |ws, window, cx| {
        ws.ungroup_tab(a, window, cx)
    });
    assert_eq!(color(cx, &workspace, a), None);
    assert_eq!(color(cx, &workspace, d), None, "one tab is no group");
}

#[gpui_kit::test]
fn a_tab_split_off_leaves_its_group_and_a_group_closes_together(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    let [a, b, c, d] = open(cx, window, &workspace, 4)[..] else {
        unreachable!()
    };
    with(cx, window, &workspace, |ws, window, cx| {
        ws.group_tab_with(b, a, window, cx);
        ws.group_tab_with(c, a, window, cx);
    });
    assert_eq!(bars(cx, &workspace), [vec![a, b, c, d]]);

    with(cx, window, &workspace, |ws, window, cx| {
        ws.split_item(c, Placement::Right, window, cx)
    });
    assert_eq!(bars(cx, &workspace), [vec![a, b, d], vec![c]]);
    assert_eq!(color(cx, &workspace, c), None);

    with(cx, window, &workspace, |ws, window, cx| {
        ws.close_tab_group(b, window, cx)
    });
    assert_eq!(bars(cx, &workspace), [vec![d], vec![c]]);
    workspace.read_with(cx, |ws, _| assert_eq!(ws.tab_groups.groups().count(), 0));
}

#[gpui_kit::test]
fn a_program_joins_the_session_on_its_host_it_was_opened_from(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    let exec: Arc<dyn HostExec> = Arc::new(NoExec);
    let target = Target::parse("user@host", None).unwrap();
    let session = SessionContext::new(target.clone(), None, true).with_exec(Some(exec.clone()));
    let spec = |key| ProgramSpec {
        title: "Logs".into(),
        host: Host {
            key,
            exec: exec.clone(),
            connected: true,
            session: None,
        },
        program: ExecRequest::new("docker"),
        shell_syntax: None,
    };
    let (other, parent) = cx
        .update_window(window, |_, window, cx| {
            let other = tab(None, cx);
            let parent = tab(Some(session), cx);
            workspace.update(cx, |ws, cx| {
                ws.add_item(parent.clone(), window, cx);
                ws.add_item(other.clone(), window, cx);
                ws.set_program_opener(|ws, _, window, cx| {
                    let program = tab(None, cx);
                    ws.add_item(program, window, cx);
                });
            });
            (other.entity_id(), parent.entity_id())
        })
        .unwrap();
    cx.run_until_parked();

    with(cx, window, &workspace, |ws, window, cx| {
        ws.activate_item_by_id(parent, window, cx);
        ws.open_program(spec(HostKey::Remote(target.clone())), window, cx);
    });
    let logs = workspace.read_with(cx, |ws, _| ws.items.last().unwrap().handle.item_id());
    assert_eq!(bars(cx, &workspace), [vec![parent, logs, other]]);
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.tab_group_items(parent), [parent, logs]);
    });

    with(cx, window, &workspace, |ws, window, cx| {
        ws.activate_item_by_id(parent, window, cx);
        ws.open_program(spec(HostKey::Local), window, cx);
    });
    let local = workspace.read_with(cx, |ws, _| ws.items.last().unwrap().handle.item_id());
    assert_eq!(
        color(cx, &workspace, local),
        None,
        "another host's program stands alone"
    );
}
