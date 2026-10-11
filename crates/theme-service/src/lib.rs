//! Shared theme library admission and catalog publication without views.
//!
//! Owns: bounded mutations, per-library busy state and coalesced reload epochs.
//! Does not own: filesystem, HTTP, settings selection, palettes or views.
//! Ports used: ThemeLibrary, ThemeRegistry. Ports provided: ThemesHandle.
//! Settings section: none. Globals: none.
//! Invariants: publish committed catalog before releasing admission or receipt.
use futures::channel::oneshot;
use gpui::{App, AppContext as _, Context, Entity, Subscription, Task, Window};
use nocterm_core::persist::WriteGate;
use nocterm_themes::{
    ExtensionInfo, ThemeCatalog, ThemeChange, ThemeLibrary, ThemeOperationError, ThemeRegistry,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
#[derive(Clone)]
pub struct ThemesHandle(Entity<ThemesState>);
struct ThemesState {
    library: Option<Arc<dyn ThemeLibrary>>,
    registry: Option<Arc<dyn ThemeRegistry>>,
    catalog: Arc<ThemeCatalog>,
    operations: Operations,
    reload: Reload,
    gate: WriteGate,
    epoch: u64,
}
#[derive(Default)]
struct Operations {
    busy: BTreeSet<String>,
    errors: BTreeMap<String, Arc<ThemeOperationError>>,
}
#[derive(Default)]
struct Reload {
    active: bool,
    pending: bool,
    generation: u64,
}
impl ThemesHandle {
    pub fn install(
        library: Option<Arc<dyn ThemeLibrary>>,
        registry: Option<Arc<dyn ThemeRegistry>>,
        catalog: ThemeCatalog,
        cx: &mut App,
    ) -> Self {
        Self(cx.new(|_| ThemesState {
            library,
            registry,
            catalog: Arc::new(catalog),
            operations: Operations::default(),
            reload: Reload::default(),
            gate: WriteGate::default(),
            epoch: 0,
        }))
    }
    pub fn catalog(&self, cx: &App) -> Arc<ThemeCatalog> {
        self.0.read(cx).catalog.clone()
    }
    pub fn registry(&self, cx: &App) -> Option<Arc<dyn ThemeRegistry>> {
        self.0.read(cx).registry.clone()
    }
    pub fn library(&self, cx: &App) -> Option<Arc<dyn ThemeLibrary>> {
        self.0.read(cx).library.clone()
    }
    pub fn busy(&self, id: &str, cx: &App) -> bool {
        self.0.read(cx).operations.busy.contains(id)
    }
    pub fn errors(&self, cx: &App) -> BTreeMap<String, Arc<ThemeOperationError>> {
        self.0.read(cx).operations.errors.clone()
    }
    pub fn observe(
        &self,
        cx: &mut App,
        mut changed: impl FnMut(&mut App) + 'static,
    ) -> Subscription {
        cx.observe(&self.0, move |_, cx| changed(cx))
    }
    pub fn observe_in<V: 'static>(
        &self,
        window: &mut Window,
        cx: &mut Context<V>,
        mut changed: impl FnMut(&mut V, &mut Window, &mut Context<V>) + 'static,
    ) -> Subscription {
        cx.observe_in(&self.0, window, move |view, _, window, cx| {
            changed(view, window, cx)
        })
    }
    pub fn install_pack(
        &self,
        extension: ExtensionInfo,
        cx: &mut App,
    ) -> Task<Result<ThemeChange, Arc<ThemeOperationError>>> {
        self.mutate(
            extension.id.clone(),
            move |library| library.install(&extension),
            cx,
        )
    }
    pub fn uninstall(
        &self,
        id: String,
        cx: &mut App,
    ) -> Task<Result<ThemeChange, Arc<ThemeOperationError>>> {
        self.mutate(id.clone(), move |library| library.uninstall(&id), cx)
    }
    fn mutate(
        &self,
        id: String,
        operation: impl FnOnce(&dyn ThemeLibrary) -> Result<ThemeChange, ThemeOperationError>
        + Send
        + 'static,
        cx: &mut App,
    ) -> Task<Result<ThemeChange, Arc<ThemeOperationError>>> {
        let admitted = self.0.update(cx, |state, cx| state.admit(&id, cx));
        let (library, gate) = match admitted {
            Ok(v) => v,
            Err(e) => {
                let error = Arc::new(e);
                if !matches!(error.as_ref(), ThemeOperationError::Busy) {
                    self.0.update(cx, |state, cx| {
                        state.operations.record_error(id, error.clone());
                        cx.notify();
                    });
                }
                return Task::ready(Err(error));
            }
        };
        let (result_tx, result_rx) = oneshot::channel();
        let (published_tx, published_rx) = oneshot::channel();
        let work = cx.background_executor().spawn(async move {
            let _guard = gate.final_write().await;
            let result = operation(library.as_ref()).map_err(Arc::new);
            let _ = result_tx.send(result);
            let _ = published_rx.await;
        });
        let (tx, rx) = oneshot::channel();
        let handle = self.clone();
        cx.spawn(async move |cx| {
            let result = result_rx
                .await
                .unwrap_or_else(|_| Err(Arc::new(ThemeOperationError::CompletionLost)));
            cx.update(|cx| {
                handle.0.update(cx, |state, cx| {
                    if let Ok(change) = &result {
                        state.catalog = Arc::new(change.catalog.clone());
                        state.epoch += 1;
                    }
                    if let Err(error) = &result {
                        state.operations.record_error(id.clone(), error.clone());
                    }
                    state.operations.busy.remove(&id);
                    cx.notify();
                });
                let reload = handle.0.read(cx).reload.pending
                    && handle.0.read(cx).operations.busy.is_empty();
                if reload {
                    handle.reload(cx);
                }
                let _ = published_tx.send(());
                let _ = tx.send(result);
            });
            work.await;
        })
        .detach();
        cx.spawn(async move |_| {
            rx.await
                .unwrap_or_else(|_| Err(Arc::new(ThemeOperationError::CompletionLost)))
        })
    }
    pub fn reload(&self, cx: &mut App) {
        let scan = self.0.update(cx, |state, _| {
            if state.reload.active || !state.operations.busy.is_empty() {
                state.reload.pending = true;
                return None;
            }
            let library = state.library.clone()?;
            state.reload.active = true;
            state.reload.pending = false;
            state.reload.generation += 1;
            Some((library, state.epoch, state.reload.generation))
        });
        let Some((library, epoch, generation)) = scan else {
            return;
        };
        let work = cx
            .background_executor()
            .spawn(async move { library.scan() });
        let handle = self.clone();
        cx.spawn(async move |cx| {
            let catalog = work.await;
            cx.update(|cx| {
                let repeat = handle.0.update(cx, |state, cx| {
                    if state.reload.generation == generation {
                        state.reload.active = false;
                        if state.epoch == epoch && state.operations.busy.is_empty() {
                            state.catalog = Arc::new(catalog);
                            cx.notify();
                        } else {
                            state.reload.pending = true;
                        }
                    }
                    state.reload.pending && state.operations.busy.is_empty()
                });
                if repeat {
                    handle.reload(cx);
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests;

impl ThemesState {
    fn admit(
        &mut self,
        id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(Arc<dyn ThemeLibrary>, WriteGate), ThemeOperationError> {
        let library = self
            .library
            .clone()
            .ok_or(ThemeOperationError::Unsupported)?;
        if self.operations.busy.contains(id) {
            return Err(ThemeOperationError::Busy);
        }
        if self.operations.busy.len() >= 32 {
            return Err(ThemeOperationError::Full);
        }
        self.operations.busy.insert(id.to_owned());
        self.operations.errors.remove(id);
        self.epoch += 1;
        cx.notify();
        Ok((library, self.gate.clone()))
    }
}

impl Operations {
    fn record_error(&mut self, id: String, error: Arc<ThemeOperationError>) {
        if !self.errors.contains_key(&id) && self.errors.len() >= 32 {
            self.errors.pop_first();
        }
        self.errors.insert(id, error);
    }
}
