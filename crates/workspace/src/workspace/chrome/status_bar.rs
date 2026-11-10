//! The status bar: the widgets registered for it and the row that draws the arrangement.
//!
//! [`StatusBar`] holds each registered widget's factory and creates the view only while the
//! arrangement places the widget. [`BarInputs`] resolves the arrangement against the window's
//! state, and [`StatusBarRow`] draws it.

use std::{collections::HashSet, rc::Rc};

use gpui_kit::{AnyView, App, Context, Window};
use nocterm_plugin_model::{ContributionId, Zone};
use nocterm_window_layout::{ResolvedItem, ResolvedLayout};

use crate::Workspace;

mod inputs;
mod menu;
mod overflow;
mod row;

#[cfg(any(test, feature = "test-support"))]
pub(crate) use inputs::ToggleLooks;
pub(super) use inputs::{BarEntry, BarInputs};
pub(super) use menu::BarMenu;
use menu::Pointed;
pub(super) use overflow::BarOverflow;
pub(super) use row::footer;

/// Creates the view of a widget.
pub(in crate::workspace) type WidgetFactory =
    Rc<dyn Fn(&Workspace, &mut Window, &mut Context<Workspace>) -> AnyView>;

/// Tells a widget's view the zone it is placed in.
pub(in crate::workspace) type ZoneHook = Rc<dyn Fn(&AnyView, Zone, &mut App)>;
/// Receives actual bar/overflow visibility after composition, outside the shell borrow.
pub(in crate::workspace) type VisibilityHook = Rc<dyn Fn(&AnyView, bool, &mut App)>;

struct WidgetEntry {
    id: ContributionId,
    factory: WidgetFactory,
    /// For a widget that follows its zone.
    zone_hook: Option<ZoneHook>,
    visibility_hook: Option<VisibilityHook>,
    visible: bool,
    /// `None` while the arrangement does not place the widget.
    view: Option<AnyView>,
    /// The zone the view was last told about.
    zone: Option<Zone>,
}

impl WidgetEntry {
    fn visibility(&mut self, visible: bool, cx: &mut App) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        if let (Some(view), Some(hook)) = (&self.view, &self.visibility_hook) {
            let (view, hook) = (view.clone(), hook.clone());
            cx.defer(move |cx| hook(&view, visible, cx));
        }
    }
}

/// The registered status bar widgets, in registration order.
pub(in crate::workspace) struct StatusBar {
    widgets: Vec<WidgetEntry>,
    /// The entry the pointer was last pressed on, for the context menu.
    pointed: Pointed,
    /// What the row measured and whether the overflow menu is open.
    overflow: BarOverflow,
}

impl StatusBar {
    pub(in crate::workspace) fn new() -> Self {
        Self {
            widgets: Vec::new(),
            pointed: Pointed::default(),
            overflow: BarOverflow::default(),
        }
    }

    pub(in crate::workspace) fn overflow(&self) -> &BarOverflow {
        &self.overflow
    }

    pub(in crate::workspace) fn pointed(&self) -> &Pointed {
        &self.pointed
    }

    /// Registers a widget; its view comes with [`StatusBar::fill`].
    pub(in crate::workspace) fn add(
        &mut self,
        id: ContributionId,
        factory: WidgetFactory,
        zone_hook: Option<ZoneHook>,
        visibility_hook: Option<VisibilityHook>,
    ) {
        self.widgets.push(WidgetEntry {
            id,
            factory,
            zone_hook,
            visibility_hook,
            visible: false,
            view: None,
            zone: None,
        });
    }

    /// Forgets a widget, so its plugin's uninstall or replacement leaves no view behind.
    pub(in crate::workspace) fn remove(&mut self, id: &ContributionId, cx: &mut App) {
        if let Some(entry) = self.widgets.iter_mut().find(|entry| entry.id == *id) {
            entry.visibility(false, cx);
        }
        self.widgets.retain(|entry| entry.id != *id);
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

    /// Draws the widgets again: the bar caches them, and a widget may show the
    /// workspace's state.
    pub(in crate::workspace) fn redraw_widgets(&self, cx: &mut App) {
        for view in self.widgets.iter().filter_map(|entry| entry.view.as_ref()) {
            cx.notify(view.entity_id());
        }
    }

    /// Keeps the view a factory created.
    pub(in crate::workspace) fn fill(&mut self, id: &ContributionId, view: AnyView) {
        if let Some(entry) = self.widgets.iter_mut().find(|entry| entry.id == *id) {
            entry.view = Some(view);
        }
    }

    /// Drops the views of the widgets the arrangement does not place.
    pub(in crate::workspace) fn release_unplaced(&mut self, layout: &ResolvedLayout, cx: &mut App) {
        for entry in &mut self.widgets {
            if !layout.is_placed(&entry.id) {
                entry.visibility(false, cx);
                entry.view = None;
                entry.zone = None;
            }
        }
    }

    pub(in crate::workspace) fn publish_visibility(
        &mut self,
        visible: &HashSet<ContributionId>,
        cx: &mut App,
    ) {
        for entry in &mut self.widgets {
            entry.visibility(entry.view.is_some() && visible.contains(&entry.id), cx);
        }
    }

    /// Tells the widgets that follow their zone where the arrangement places them now.
    pub(in crate::workspace) fn follow_zones(&mut self, layout: &ResolvedLayout, cx: &mut App) {
        for entry in &mut self.widgets {
            let (Some(view), Some(hook)) = (&entry.view, &entry.zone_hook) else {
                continue;
            };
            let Some(zone) = zone_of(layout, &entry.id) else {
                continue;
            };
            if entry.zone != Some(zone) {
                entry.zone = Some(zone);
                hook(view, zone, cx);
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

/// The zone of the bar that places the widget.
fn zone_of(layout: &ResolvedLayout, id: &ContributionId) -> Option<Zone> {
    [Zone::Left, Zone::Center, Zone::Right]
        .into_iter()
        .find(|zone| {
            layout
                .zone(*zone)
                .iter()
                .any(|item| matches!(item, ResolvedItem::Widget(widget) if widget == id))
        })
}
