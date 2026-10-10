//! Typed subscription installation; callbacks route identities and immutable input.
use super::*;
use gpui_kit::WeakEntity;

pub(super) struct WorkspaceBindings {
    workspace: WeakEntity<Workspace>,
}

pub(super) struct PreparedTab {
    handle: Rc<dyn ItemHandle>,
    session: Option<SessionContext>,
    dock_item: Entity<crate::dock_item::DockItem>,
    local: Option<Box<dyn crate::local_terminal::LocalTerminalHandle>>,
    cwd: Option<PathBuf>,
    subscription: Subscription,
    focus_subscription: Subscription,
}

impl PreparedTab {
    pub(super) fn into_entry(self) -> super::tabs::TabEntry {
        super::tabs::TabEntry::new(
            self.handle,
            self.session,
            self.dock_item,
            self.local,
            self.cwd,
            self.subscription,
            self.focus_subscription,
        )
    }
}

impl WorkspaceBindings {
    pub(super) fn new(workspace: WeakEntity<Workspace>) -> Self {
        Self { workspace }
    }

    pub(super) fn prepare<T: Item>(
        &self,
        item: Entity<T>,
        local: Option<Box<dyn crate::local_terminal::LocalTerminalHandle>>,
        origin: Rc<Cell<Option<EntityId>>>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> PreparedTab {
        let id = item.entity_id();
        let cwd = local.as_ref().and_then(|local| local.cwd(cx));
        let subscription =
            cx.subscribe_in(
                &item,
                window,
                move |this, item, event, window, cx| match event {
                    ItemEvent::Changed => this.item_changed(id, item.read(cx).session(cx), cx),
                    ItemEvent::CloseRequested => this.close_item_by_id(id, window, cx),
                },
            );
        let is_local = local.is_some();
        let session = item.read(cx).session(cx);
        let handle: Rc<dyn ItemHandle> = Rc::new(item);
        let workspace = self.workspace.clone();
        let dock_item = cx.new(|cx| {
            crate::dock_item::DockItem::new(handle.clone(), workspace, is_local, origin, cx)
        });
        let focus = dock_item.read(cx).container_focus_handle();
        let focus_subscription = cx.on_focus_in(&focus, window, move |this, _, cx| {
            this.item_focused(id, cx);
        });
        PreparedTab {
            handle,
            session,
            dock_item,
            local,
            cwd,
            subscription,
            focus_subscription,
        }
    }
}
