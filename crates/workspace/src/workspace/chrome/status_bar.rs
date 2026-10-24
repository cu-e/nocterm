//! The status bar: the widgets registered for it and the row that draws the arrangement.
//!
//! [`StatusBar`] holds each registered widget's factory and creates the view only while the
//! arrangement places the widget. [`BarInputs`] resolves the arrangement against the window's
//! state, and [`StatusBarRow`] draws it.

use std::rc::Rc;

use gpui_kit::{AnyView, Context, Window};
use nocterm_plugin_model::ContributionId;
use nocterm_window_layout::ResolvedLayout;

use crate::Workspace;

mod inputs;
mod row;

#[cfg(any(test, feature = "test-support"))]
pub(crate) use inputs::ToggleLooks;
pub(super) use inputs::{BarEntry, BarInputs};
pub(super) use row::footer;

/// Creates the view of a widget.
pub(in crate::workspace) type WidgetFactory =
    Rc<dyn Fn(&Workspace, &mut Window, &mut Context<Workspace>) -> AnyView>;

struct WidgetEntry {
    id: ContributionId,
    factory: WidgetFactory,
    /// `None` while the arrangement does not place the widget.
    view: Option<AnyView>,
}

/// The registered status bar widgets, in registration order.
pub(in crate::workspace) struct StatusBar {
    widgets: Vec<WidgetEntry>,
}

impl StatusBar {
    pub(in crate::workspace) fn new() -> Self {
        Self {
            widgets: Vec::new(),
        }
    }

    /// Registers a widget; its view comes with [`StatusBar::fill`].
    pub(in crate::workspace) fn add(&mut self, id: ContributionId, factory: WidgetFactory) {
        self.widgets.push(WidgetEntry {
            id,
            factory,
            view: None,
        });
    }

    /// The widgets the arrangement places and that have no view yet.
    pub(in crate::workspace) fn pending(
        &self,
        layout: &ResolvedLayout,
    ) -> Vec<(ContributionId, WidgetFactory)> {
        self.widgets
            .iter()
            .filter(|entry| entry.view.is_none() && layout.is_placed(&entry.id))
            .map(|entry| (entry.id.clone(), entry.factory.clone()))
            .collect()
    }

    /// Keeps the view a factory created.
    pub(in crate::workspace) fn fill(&mut self, id: &ContributionId, view: AnyView) {
        if let Some(entry) = self.widgets.iter_mut().find(|entry| entry.id == *id) {
            entry.view = Some(view);
        }
    }

    /// Drops the views of the widgets the arrangement does not place.
    pub(in crate::workspace) fn release_unplaced(&mut self, layout: &ResolvedLayout) {
        for entry in &mut self.widgets {
            if !layout.is_placed(&entry.id) {
                entry.view = None;
            }
        }
    }

    pub(in crate::workspace) fn view(&self, id: &ContributionId) -> Option<&AnyView> {
        self.widgets
            .iter()
            .find(|entry| entry.id == *id)
            .and_then(|entry| entry.view.as_ref())
    }
}
