use super::*;
use gpui_kit::{
    Bounds, InputEvent, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, ScrollDelta, point, px,
};
use nocterm_workspace::WorkspaceEvent;

struct PanelHost(Entity<AgentPanel>);

impl gpui_kit::Render for PanelHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::div().size_full().child(self.0.clone())
    }
}

fn many_servers(f: &Fixture, cx: &mut TestAppContext) -> Rc<Directory> {
    let directory = Rc::new(Directory(std::cell::RefCell::new(
        (0..35)
            .map(|index| super::super::servers::summary(&format!("server-{index:02}"), None))
            .collect(),
    )));
    cx.update(|cx| {
        f.workspace.update(cx, |workspace, _| {
            workspace.set_connection_directory(directory.clone())
        });
    });
    directory
}

const ROW: &str = "attach-connection-server-10";

fn scrolled_picker(f: &Fixture, cx: &mut TestAppContext) -> Bounds<Pixels> {
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-attach-context", cx);
        let initial = window.find(ROW).bounds();
        window.scroll(
            "agent-picker",
            ScrollDelta::Pixels(point(px(0.), px(-240.))),
            cx,
        );
        let scrolled = window.find(ROW);
        assert!(scrolled.visible());
        assert!(scrolled.bounds().origin.y < initial.origin.y);
        window.hover(ROW, cx);
        scrolled.bounds()
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_scrolled_attachment_picker_keeps_its_rows_and_accepts_selection_after_refresh(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let directory = many_servers(&f, cx);
    new_chat(&f, cx);
    // Exercise the footer's own cache without a workspace ancestor retaining its state.
    cx.update_window(f.handle, |_, window, cx| {
        window.replace_root(cx, |_, _| PanelHost(f.panel.clone()));
    })
    .unwrap();
    let before = scrolled_picker(&f, cx);
    for event in [
        WorkspaceEvent::ItemsChanged,
        WorkspaceEvent::ActiveSessionChanged,
        WorkspaceEvent::ConnectionsChanged,
    ] {
        for _ in 0..3 {
            cx.update_window(f.handle, |_, window, cx| {
                window.draw(cx).clear(cx);
                assert_eq!(window.find(ROW).bounds(), before);
            })
            .unwrap();
        }
        cx.update(|cx| f.workspace.update(cx, |_, cx| cx.emit(event)));
        cx.run_until_parked();
        cx.update_window(f.handle, |_, window, cx| {
            window.draw(cx).clear(cx);
            assert_eq!(window.find(ROW).bounds(), before);
        })
        .unwrap();
    }
    directory
        .0
        .borrow_mut()
        .push(super::super::servers::summary("server-zz", None));
    cx.update(|cx| {
        f.workspace
            .update(cx, |_, cx| cx.emit(WorkspaceEvent::ConnectionsChanged))
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(window.find(ROW).bounds(), before);
        assert_eq!(
            window.find("attach-connection-server-zz").label(),
            Some("server-zz")
        );
        window.click(ROW, cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(
            thread
                .read(cx)
                .composer()
                .attachments()
                .contains(&Attachment::Connection("server-10".into()))
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_attachment_click_survives_a_session_update_between_press_and_release(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    many_servers(&f, cx);
    new_chat(&f, cx);
    let position = scrolled_picker(&f, cx).center();
    cx.update_window(f.handle, |_, window, cx| {
        window.dispatch_event(
            MouseDownEvent {
                position,
                button: MouseButton::Left,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.draw(cx).clear(cx);
    })
    .unwrap();
    cx.update(|cx| {
        f.workspace
            .update(cx, |_, cx| cx.emit(WorkspaceEvent::ActiveSessionChanged))
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(window.find(ROW).bounds().center(), position);
        window.dispatch_event(
            MouseUpEvent {
                position,
                button: MouseButton::Left,
                modifiers: Default::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
        window.draw(cx).clear(cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(
            thread
                .read(cx)
                .composer()
                .attachments()
                .contains(&Attachment::Connection("server-10".into()))
        );
        window.press("escape", cx);
        assert!(window.try_find("agent-picker").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn reopening_the_attachment_picker_starts_at_the_top(cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 18., workspace_resolver(), image_deps());
    many_servers(&f, cx);
    new_chat(&f, cx);
    scrolled_picker(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.press("escape", cx);
        window.draw(cx).clear(cx);
        window.click("agent-attach-context", cx);
        let heading = window.find("attach-local-terminal");
        assert!(heading.visible());
        assert!(heading.bounds().origin.y >= window.find("agent-picker").bounds().origin.y);
        window.click("attach-connection-server-00", cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(
            thread
                .read(cx)
                .composer()
                .attachments()
                .contains(&Attachment::Connection("server-00".into()))
        );
    })
    .unwrap();
}
