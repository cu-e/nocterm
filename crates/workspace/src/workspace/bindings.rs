//! Typed subscription installation; callbacks route identities and immutable input.
use super::*;

pub(super) struct PreparedTab {
    handle: Rc<dyn ItemHandle>,
    session: Option<SessionContext>,
    dock_item: Entity<crate::dock_item::DockItem>,
    local: Option<Box<dyn crate::local_terminal::LocalTerminalHandle>>,
    cwd: Option<PathBuf>,
    subscription: Subscription,
    focus_subscription: Subscription,
    project: Option<project_surface::ProjectPresentation>,
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
        .with_project(self.project)
    }
}

pub(super) fn prepare_tab<T: Item>(
    item: Entity<T>,
    local: Option<Box<dyn crate::local_terminal::LocalTerminalHandle>>,
    origin: Rc<Cell<Option<EntityId>>>,
    project: Option<project_surface::ProjectPresentation>,
    source: ProjectSurfaceReceipt,
    window: &mut Window,
    cx: &mut Context<surface::TabSurface>,
) -> PreparedTab {
    let id = item.entity_id();
    let cwd = local.as_ref().and_then(|local| local.cwd(cx));
    let subscription =
        cx.subscribe_in(
            &item,
            window,
            move |this, item, event, window, cx| match event {
                ItemEvent::Changed => {
                    this.item_changed_at(source, id, item.read(cx).session(cx), cx)
                }
                ItemEvent::CloseRequested => this.close_item_at(source, id, window, cx),
            },
        );
    let is_local = local.is_some();
    let session = item.read(cx).session(cx);
    let handle: Rc<dyn ItemHandle> = Rc::new(item);
    let workspace = cx.weak_entity();
    let project_view = project
        .as_ref()
        .map(project_surface::ProjectPresentation::surface);
    let dock_item = cx.new(|cx| {
        crate::dock_item::DockItem::new(
            handle.clone(),
            workspace,
            source,
            is_local,
            origin,
            project_view,
            cx,
        )
    });
    let focus = dock_item.read(cx).container_focus_handle();
    let focus_subscription = cx.on_focus_in(&focus, window, move |this, _, cx| {
        this.item_focused_at(source, id, cx);
    });
    PreparedTab {
        handle,
        session,
        dock_item,
        local,
        cwd,
        subscription,
        focus_subscription,
        project,
    }
}

/// Install native events at the entity boundary, then hand lifetimes to the owner.
pub(super) fn prepare_topology(
    source: ProjectSurfaceReceipt,
    window: &mut Window,
    cx: &mut Context<surface::TabSurface>,
) -> tabs::topology::DockTopology {
    let (dock, skin) = if source.context() == nocterm_project_model::ContextId::General {
        DockSkin::dock_area(source.dock_id(), None, window, cx)
    } else {
        tabs::project_skin::ProjectDockSkin::dock_area(source.dock_id(), window, cx)
    };
    skin.set_panel_style(PanelStyle::TabBar, cx);
    skin.set_close_button_visible(false, cx);
    skin.set_toggle_button_visible(false, cx);
    dock.update(cx, |dock, cx| {
        dock.set_closed_bottom_strip_visible(false, cx);
        dock.set_last_panel_drag_between_regions(true, window, cx);
    });
    let events = cx.subscribe_in(&dock, window, move |this, _, _: &DockEvent, window, cx| {
        this.native_dock_changed(source, window, cx)
    });
    let observer = cx.observe_in(&dock, window, move |this, _, window, cx| {
        this.native_dock_visibility_changed(source, window, cx)
    });
    tabs::topology::DockTopology::new(dock, events, observer)
}
