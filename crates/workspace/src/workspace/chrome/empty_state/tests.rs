use std::{cell::RefCell, rc::Rc, sync::Arc};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{App, EntityId, Image, ImageFormat, TestAppContext, WeakEntity, Window};
use nocterm_session::Target;

use super::*;
use crate::ConnectionDirectory;

struct RecentDirectory {
    recent: Vec<ConnectionSummary>,
    opened: Rc<RefCell<Vec<String>>>,
}

impl ConnectionDirectory for RecentDirectory {
    fn connections(&self, _: &App) -> Vec<ConnectionSummary> {
        self.recent.clone()
    }

    fn recent_connections(&self, _: &App) -> Vec<ConnectionSummary> {
        self.recent.clone()
    }

    fn open(
        &self,
        id: &str,
        workspace: &WeakEntity<Workspace>,
        _: &mut Window,
        cx: &mut App,
    ) -> bool {
        self.opened.borrow_mut().push(id.into());
        // A real Directory updates Workspace. This must not reborrow a listener's update.
        workspace
            .update(cx, |workspace, cx| workspace.show_new_tab_menu(true, cx))
            .unwrap();
        true
    }

    fn open_background(
        &self,
        _: &str,
        _: &WeakEntity<Workspace>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<EntityId> {
        None
    }
}

fn connection(index: usize) -> ConnectionSummary {
    ConnectionSummary {
        id: format!("id-{index}").into(),
        name: format!("Server {index}").into(),
        group: None,
        description: "Description must not appear".into(),
        target: Target::new("root", format!("192.0.2.{index}"), 22),
        icon: None,
        flag: Some(Arc::new(Image::from_bytes(
            ImageFormat::Svg,
            b"<svg xmlns='http://www.w3.org/2000/svg' width='16' height='11'><path fill='red' d='M0 0H16V11H0Z'/></svg>".to_vec(),
        ))),
    }
}

#[gpui_kit::test]
fn empty_center_limits_recent_rows_and_reconnects_exact_saved_id(cx: &mut TestAppContext) {
    let (handle, workspace) = crate::workspace::tests::fixture(cx);
    let opened = Rc::new(RefCell::new(Vec::new()));
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, _| {
            workspace.set_connection_directory(Rc::new(RecentDirectory {
                recent: (0..8).map(connection).collect(),
                opened: opened.clone(),
            }));
        });
        window.resize(gpui_kit::size(px(640.), px(400.)));
        window.render_frame(cx);
        let button = window.find("empty-new-tab");
        assert_eq!(button.label(), Some("New Connection"));
        for index in 0..RECENT_LIMIT {
            let row = window.find(SharedString::from(format!("empty-recent-id-{index}")));
            assert_eq!(row.label(), Some(format!("Server {index}").as_str()));
            assert!(row.bounds().size.height < px(36.), "one compact line");
            assert!(row.bounds().top() > button.bounds().bottom());
            let flag = window.find(SharedString::from(format!("empty-recent-flag-id-{index}")));
            assert!(flag.bounds().left() > row.bounds().center().x);
        }
        assert!(window.try_find("empty-recent-id-6").is_none());
        assert!(window.try_find("empty-recent-id-7").is_none());
        let list = window.find("empty-recent-connections");
        assert!(list.bounds().bottom() < window.viewport_size().height);
        window.click("empty-recent-id-2", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(&*opened.borrow(), &[String::from("id-2")]);
    assert!(workspace.read_with(cx, |workspace, _| workspace.new_tab_menu_open));
}

#[gpui_kit::test]
fn empty_center_new_connection_preserves_new_tab_action_without_recent_servers(
    cx: &mut TestAppContext,
) {
    let (handle, workspace) = crate::workspace::tests::fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        let focus = workspace.read(cx).focus_handle.clone();
        window.focus(&focus, cx);
        window.render_frame(cx);
        assert_eq!(window.find("empty-new-tab").label(), Some("New Connection"));
        assert!(window.try_find("empty-recent-connections").is_none());
        window.click("empty-new-tab", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(workspace.read_with(cx, |workspace, _| workspace.new_tab_menu_open));
}
