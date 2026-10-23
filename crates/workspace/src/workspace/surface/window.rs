//! Window-local source routing and UI selection; no Item or pane-order mirror.
use super::*;
use gpui_kit::WeakEntity;

pub(in crate::workspace) struct SurfaceWindow {
    owner: ContextWindowReceipt,
    sources: std::cell::RefCell<Vec<WeakEntity<TabSurface>>>,
    generation: Cell<u64>,
    closed: Cell<bool>,
}
impl SurfaceWindow {
    pub(in crate::workspace) fn new(owner: ContextWindowReceipt) -> Rc<Self> {
        Rc::new(Self {
            owner,
            sources: Default::default(),
            generation: Cell::new(0),
            closed: Cell::new(false),
        })
    }
    pub(in crate::workspace) fn owner(&self) -> ContextWindowReceipt {
        self.owner
    }
    pub(in crate::workspace) fn register_outer(&self, entity: &Entity<TabSurface>) {
        self.sources.borrow_mut().push(entity.downgrade());
    }
    pub(in crate::workspace) fn create_project(
        self: &Rc<Self>,
        project: nocterm_project_model::ProjectId,
        outer: EntityId,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Entity<TabSurface>, ContextSelectionError> {
        self.check_window(window)?;
        self.sources.borrow_mut().retain(|source| {
            source
                .upgrade()
                .is_some_and(|source| !source.read(cx).selection.closed())
        });
        if self.sources.borrow().len() > nocterm_project_model::MAX_PROJECTS {
            return Err(ContextSelectionError::Capacity);
        }
        let entity = cx.new(|cx| {
            TabSurface::new(
                self.clone(),
                ContextId::Project(project),
                Some(outer),
                window,
                cx,
            )
        });
        self.sources.borrow_mut().push(entity.downgrade());
        Ok(entity)
    }
    pub(in crate::workspace) fn check_window(
        &self,
        window: &Window,
    ) -> Result<(), ContextSelectionError> {
        if self.closed.get() || !self.owner.matches(window) {
            return Err(ContextSelectionError::StaleWindow);
        }
        Ok(())
    }
    pub(in crate::workspace) fn generation(&self) -> u64 {
        self.generation.get()
    }
    pub(in crate::workspace) fn available(&self) -> bool {
        !self.closed.get()
    }
    pub(in crate::workspace) fn presented(&self, source: ProjectSurfaceReceipt, cx: &App) -> bool {
        if !self.registered(source, cx) {
            return false;
        }
        if source.context() == ContextId::General {
            return true;
        }
        self.sources
            .borrow()
            .first()
            .and_then(WeakEntity::upgrade)
            .is_some_and(|outer| {
                source
                    .outer_item()
                    .is_some_and(|item| outer.read(cx).tabs().item_displayed(item, cx))
            })
    }
    pub(in crate::workspace) fn registered(&self, source: ProjectSurfaceReceipt, cx: &App) -> bool {
        if !self.available() {
            return false;
        }
        if source.context() == ContextId::General {
            return true;
        }
        let Some(outer_item) = source.outer_item() else {
            return false;
        };
        let Some(outer) = self.sources.borrow().first().and_then(WeakEntity::upgrade) else {
            return false;
        };
        let outer = outer.read(cx);
        outer
            .tabs()
            .project(outer_item)
            .is_some_and(|project| project.engine().entity_id() == source.surface_id())
    }
    pub(in crate::workspace) fn select_outer(&self, source: ProjectSurfaceReceipt, cx: &mut App) {
        let Some(generation) = self.generation().checked_add(1) else {
            self.closed.set(true);
            return;
        };
        self.generation.set(generation);
        let sources = self.sources.borrow().clone();
        for entity in sources
            .into_iter()
            .filter(|entity| entity.entity_id() != source.surface_id())
            .filter_map(|entity| entity.upgrade())
        {
            entity.update(cx, |_, cx| cx.notify());
        }
    }
    pub(in crate::workspace) fn foreign_item(
        &self,
        original: ProjectSurfaceReceipt,
        item: EntityId,
        cx: &App,
    ) -> bool {
        self.sources
            .borrow()
            .iter()
            .filter(|source| source.entity_id() != original.surface_id())
            .filter_map(WeakEntity::upgrade)
            .any(|source| source.read(cx).contains_item(item))
    }
    pub(in crate::workspace) fn resolve(
        &self,
        source: ProjectSurfaceReceipt,
        cx: &App,
    ) -> Result<Entity<TabSurface>, ContextSelectionError> {
        if self.closed.get() || source.window() != self.owner {
            return Err(ContextSelectionError::StaleWindow);
        }
        let entity = self
            .sources
            .borrow()
            .iter()
            .find(|entity| entity.entity_id() == source.surface_id())
            .and_then(WeakEntity::upgrade)
            .ok_or(ContextSelectionError::StaleSurface)?;
        if entity.read(cx).selection.closed() || entity.read(cx).source() != source {
            return Err(ContextSelectionError::StaleSurface);
        }
        Ok(entity)
    }
    pub(in crate::workspace) fn retire(&self) {
        self.closed.set(true);
    }
}
