//! Panels and widgets registered under the ids of the plugin catalog.
//!
//! An owner names its contribution and hands the workspace a factory. The workspace checks the
//! id against the catalog its [`Deps`](super::Deps) carry, creates the view and places it
//! according to the descriptor. This module holds free items only; the `Workspace` methods that
//! apply a registration live beside the other composition methods.

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

use gpui_kit::{AnyView, App, Context, Entity, EventEmitter, Pixels, Render, Subscription, Window};
use nocterm_plugin_model::{
    ContributionId, ContributionKind, ContributionRef, PanelChrome, PanelTraits, PluginSet, Side,
    SystemContribution, Zone,
};

use super::{
    Workspace,
    chrome::{WidgetFactory, ZoneHook},
    panel_groups::{ExpandableHandle, HostedPanel, Presentation},
};
use crate::Panel;

/// Why the workspace refused a registration.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RegistrationError {
    #[error("`{0}` is not in the plugin catalog")]
    Unlisted(ContributionId),
    #[error("`{0}` is already registered")]
    Duplicate(ContributionId),
    #[error("`{id}` is a {expected:?} in the plugin catalog")]
    KindMismatch {
        id: ContributionId,
        expected: ContributionKind,
    },
    #[error("`{0}` draws no header of its own, so its panel cannot expand")]
    NotExpandable(ContributionId),
    #[error("`{0}` can be disabled, so its panel must be created by a factory")]
    EagerNotRequired(ContributionId),
}

/// A request from an expandable panel to the group that hosts it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PanelRequest {
    /// Fill the body, or give it back.
    ToggleExpanded,
    /// Close the group.
    Close,
    /// Grow the column by this much (shrink, if negative), for a view it opened or closed.
    Widen(Pixels),
}

/// A panel that can fill the body in place of the content.
///
/// Only a panel that draws its own header (`PanelChrome::OwnHeader`) is registered with
/// [`PanelRegistration::expandable`].
pub trait ExpandablePanel: Panel + EventEmitter<PanelRequest> {
    /// The panel fills the body (`true`) or gave it back.
    fn set_expanded(&mut self, _expanded: bool, _cx: &mut Context<Self>) {}
    /// The side of the content the panel's column is on.
    fn set_side(&mut self, _side: Side, _cx: &mut Context<Self>) {}
}

type CreatePanel<T> = Box<dyn Fn(&Workspace, &mut Window, &mut Context<Workspace>) -> Entity<T>>;

/// Creates a panel for a registration and hosts it.
pub(super) type PanelFactory =
    Box<dyn Fn(&Workspace, &mut Window, &mut Context<Workspace>) -> HostedPanel>;

/// How a registered panel comes to exist.
pub(super) enum PanelSource {
    /// The panel exists already and is hosted.
    Hosted(HostedPanel),
    /// The panel is created by running the factory.
    Factory(PanelFactory),
}
type Host<T> = fn(Entity<T>, PanelChrome, &mut Window, &mut Context<Workspace>) -> HostedPanel;

enum Origin<T> {
    Factory(CreatePanel<T>),
    Entity(Entity<T>),
}

/// A panel to register for a contribution.
pub struct PanelRegistration<T> {
    id: ContributionId,
    origin: Origin<T>,
    host: Host<T>,
    expandable: bool,
}

impl<T: Panel> PanelRegistration<T> {
    /// The workspace calls `factory` each time it creates the panel: at registration while its
    /// plugin is enabled, and again whenever the plugin is enabled after being disabled. It
    /// borrows itself to the factory: the factory may read the workspace but cannot update it,
    /// and it must not keep state that has to survive a call.
    ///
    /// The descriptor is read for its id only: the catalog holds the checked descriptor.
    pub fn new(
        contribution: &SystemContribution,
        factory: impl Fn(&Workspace, &mut Window, &mut Context<Workspace>) -> Entity<T> + 'static,
    ) -> Self {
        Self::of(contribution, Origin::Factory(Box::new(factory)))
    }

    /// Registers a panel that exists already. Only core contributions and required plugins may,
    /// because a panel the workspace did not create cannot be dropped when its plugin is
    /// disabled.
    pub fn with_entity(contribution: &SystemContribution, panel: Entity<T>) -> Self {
        Self::of(contribution, Origin::Entity(panel))
    }

    /// As [`PanelRegistration::new`], for a panel of an installed user plugin. The panel is
    /// always created by a factory and never expands.
    pub fn user(
        id: &ContributionId,
        factory: impl Fn(&Workspace, &mut Window, &mut Context<Workspace>) -> Entity<T> + 'static,
    ) -> Self {
        Self::named(id.clone(), Origin::Factory(Box::new(factory)))
    }

    fn of(contribution: &SystemContribution, origin: Origin<T>) -> Self {
        Self::named(contribution.id.clone(), origin)
    }

    fn named(id: ContributionId, origin: Origin<T>) -> Self {
        Self {
            id,
            origin,
            host: host_plain::<T>,
            expandable: false,
        }
    }

    pub(super) fn id(&self) -> &ContributionId {
        &self.id
    }
    pub(super) fn expandable_requested(&self) -> bool {
        self.expandable
    }
    pub(super) fn is_eager(&self) -> bool {
        matches!(self.origin, Origin::Entity(_))
    }

    /// Hosts the panel now if it exists; otherwise gives the factory that creates and hosts it.
    pub(super) fn into_source(
        self,
        chrome: PanelChrome,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> PanelSource {
        let host = self.host;
        match self.origin {
            Origin::Entity(panel) => PanelSource::Hosted(host(panel, chrome, window, cx)),
            Origin::Factory(create) => {
                PanelSource::Factory(Box::new(move |workspace, window, cx| {
                    let panel = create(workspace, window, cx);
                    host(panel, chrome, window, cx)
                }))
            }
        }
    }
}

impl<T: ExpandablePanel> PanelRegistration<T> {
    /// The panel can fill the body. Its contribution must draw its own header.
    pub fn expandable(mut self) -> Self {
        self.host = host_expandable::<T>;
        self.expandable = true;
        self
    }
}

/// A widget whose popovers open from the zone of the status bar that places it.
///
/// The workspace tells the view its zone when it creates the view and whenever a layout change
/// moves the widget to another zone, so a popover can open toward the inside of the window.
pub trait ZoneAware: Render {
    /// The widget is placed in `zone`.
    fn set_zone(&mut self, zone: Zone, cx: &mut Context<Self>);
}

/// A status bar widget to register for a contribution.
pub struct WidgetRegistration {
    id: ContributionId,
    create: WidgetFactory,
    zone: Option<ZoneHook>,
}

impl WidgetRegistration {
    /// The workspace calls `factory` when the window's arrangement places the widget, and
    /// borrows itself to it: the factory may read the workspace but cannot update it. A widget
    /// that the arrangement leaves out is not created.
    pub fn new<V: Render>(
        contribution: &SystemContribution,
        factory: impl Fn(&Workspace, &mut Window, &mut Context<Workspace>) -> Entity<V> + 'static,
    ) -> Self {
        Self::user(&contribution.id, factory)
    }

    /// As [`WidgetRegistration::new`], for a widget of an installed user plugin.
    pub fn user<V: Render>(
        id: &ContributionId,
        factory: impl Fn(&Workspace, &mut Window, &mut Context<Workspace>) -> Entity<V> + 'static,
    ) -> Self {
        Self {
            id: id.clone(),
            create: Rc::new(move |workspace, window, cx| factory(workspace, window, cx).into()),
            zone: None,
        }
    }

    /// As [`WidgetRegistration::new`], for a view that follows the zone it is placed in.
    pub fn zone_aware<V: ZoneAware>(
        contribution: &SystemContribution,
        factory: impl Fn(&Workspace, &mut Window, &mut Context<Workspace>) -> Entity<V> + 'static,
    ) -> Self {
        let mut registration = Self::new(contribution, factory);
        registration.zone = Some(Rc::new(|view: &AnyView, zone: Zone, cx: &mut App| {
            if let Ok(view) = view.clone().downcast::<V>() {
                view.update(cx, |view, cx| view.set_zone(zone, cx));
            }
        }));
        registration
    }

    pub(super) fn id(&self) -> &ContributionId {
        &self.id
    }
    pub(super) fn into_parts(self) -> (WidgetFactory, Option<ZoneHook>) {
        (self.create, self.zone)
    }
}

/// The contributions registered in a window and how to create their panels again. They are
/// checked against the plugin set in force when they register.
pub(super) struct Registrations {
    registered: HashSet<ContributionId>,
    /// How to create the panel of a registration again, by contribution id.
    factories: HashMap<ContributionId, PanelFactory>,
}

impl Registrations {
    pub(super) fn new() -> Self {
        Self {
            registered: HashSet::new(),
            factories: HashMap::new(),
        }
    }

    /// Keeps the factory of a registered panel.
    pub(super) fn keep_factory(&mut self, id: ContributionId, factory: PanelFactory) {
        self.factories.insert(id, factory);
    }

    /// How to create the panel of a registered contribution again.
    pub(super) fn factory(&self, id: &ContributionId) -> Option<&PanelFactory> {
        self.factories.get(id)
    }

    pub(super) fn is_registered(&self, id: &ContributionId) -> bool {
        self.registered.contains(id)
    }

    /// Forgets a registration, so the contribution can register again.
    #[cfg(feature = "user-plugins")]
    pub(super) fn forget(&mut self, id: &ContributionId) {
        self.registered.remove(id);
        self.factories.remove(id);
    }

    /// The set's descriptor of a contribution, if something was registered for it.
    pub(super) fn registered_contribution<'a>(
        &self,
        set: &'a PluginSet,
        id: &ContributionId,
    ) -> Option<ContributionRef<'a>> {
        self.is_registered(id)
            .then(|| set.contribution(id))
            .flatten()
    }

    /// The id of the contribution with this text, for commands that carry one.
    pub(super) fn named(&self, set: &PluginSet, name: &str) -> Option<ContributionId> {
        set.entries()
            .find(|contribution| contribution.id().as_str() == name)
            .map(|contribution| contribution.id().clone())
    }

    /// Checks a panel registration and records it. `expandable` and `eager` describe how the
    /// owner wants to host the panel.
    pub(super) fn admit_panel(
        &mut self,
        set: &PluginSet,
        id: &ContributionId,
        expandable: bool,
        eager: bool,
    ) -> Result<PanelTraits, RegistrationError> {
        let contribution = self.check(set, id, ContributionKind::Panel)?;
        // The set refuses a panel without traits, so this guard only names the kind.
        let Some(traits) = contribution.panel() else {
            return Err(RegistrationError::KindMismatch {
                id: id.clone(),
                expected: ContributionKind::Widget,
            });
        };
        if expandable && (traits.chrome != PanelChrome::OwnHeader || contribution.is_user()) {
            return Err(RegistrationError::NotExpandable(id.clone()));
        }
        let may_be_disabled = matches!(
            set.plugin_of(id),
            Some(plugin) if !plugin.required()
        );
        if eager && may_be_disabled {
            return Err(RegistrationError::EagerNotRequired(id.clone()));
        }
        self.registered.insert(id.clone());
        Ok(traits)
    }

    /// Checks a widget registration and records it.
    pub(super) fn admit_widget(
        &mut self,
        set: &PluginSet,
        id: &ContributionId,
    ) -> Result<(), RegistrationError> {
        self.check(set, id, ContributionKind::Widget)?;
        self.registered.insert(id.clone());
        Ok(())
    }

    fn check<'a>(
        &self,
        set: &'a PluginSet,
        id: &ContributionId,
        kind: ContributionKind,
    ) -> Result<ContributionRef<'a>, RegistrationError> {
        let contribution = set
            .contribution(id)
            .ok_or_else(|| RegistrationError::Unlisted(id.clone()))?;
        if contribution.kind() != kind {
            return Err(RegistrationError::KindMismatch {
                id: id.clone(),
                expected: contribution.kind(),
            });
        }
        if self.registered.contains(id) {
            return Err(RegistrationError::Duplicate(id.clone()));
        }
        Ok(contribution)
    }
}

/// A panel's title, icon and badge sit in the workspace's chrome; the rest of the panel
/// redraws itself, so only a change to them counts.
fn presentation_of<T: Panel>(panel: &Entity<T>, cx: &gpui_kit::App) -> Presentation {
    let panel = panel.read(cx);
    (panel.title(cx), panel.icon(cx), panel.badge(cx))
}

/// Whether the panel asks for the user right now.
fn attention_of<T: Panel>(panel: &Entity<T>, cx: &gpui_kit::App) -> bool {
    let panel = panel.read(cx);
    panel.attention(cx).is_some()
}

/// Keeps the chrome's copy of the panel's presentation and attention current.
fn observe_presentation<T: Panel>(panel: &Entity<T>, cx: &mut Context<Workspace>) -> Subscription {
    cx.observe(panel, move |this, panel, cx| {
        let presented = this
            .panels
            .presentation_changed(panel.entity_id(), presentation_of(&panel, cx));
        let asked = this
            .panels
            .attention_changed(panel.entity_id(), attention_of(&panel, cx));
        if presented || asked {
            cx.notify();
        }
    })
}

fn host_plain<T: Panel>(
    panel: Entity<T>,
    chrome: PanelChrome,
    _: &mut Window,
    cx: &mut Context<Workspace>,
) -> HostedPanel {
    let presentation = presentation_of(&panel, cx);
    let subscription = observe_presentation(&panel, cx);
    let attention = attention_of(&panel, cx);
    HostedPanel::plain(Box::new(panel), presentation, subscription)
        .with_chrome(chrome)
        .with_attention(attention)
}

fn host_expandable<T: ExpandablePanel>(
    panel: Entity<T>,
    chrome: PanelChrome,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> HostedPanel {
    // The group is looked up when the request arrives: the layout decides which group holds
    // the panel, and it can change.
    let requests = cx.subscribe_in(
        &panel,
        window,
        move |this, panel, request: &PanelRequest, window, cx| {
            let Some(group) = this.panels.group_holding(panel.entity_id()) else {
                return;
            };
            match request {
                PanelRequest::ToggleExpanded => this.toggle_group_expanded(group, cx),
                PanelRequest::Close => {
                    let transition = this.panels.close(group, window, cx);
                    this.apply_panel_transition(transition, window, cx);
                }
                PanelRequest::Widen(delta) => this.widen_group(group, *delta, window, cx),
            }
        },
    );
    let presentation = presentation_of(&panel, cx);
    let subscription = Subscription::join(requests, observe_presentation(&panel, cx));
    let attention = attention_of(&panel, cx);
    HostedPanel::expandable(Box::new(panel), presentation, subscription)
        .with_chrome(chrome)
        .with_attention(attention)
}

impl<T: ExpandablePanel> ExpandableHandle for Entity<T> {
    fn set_expanded(&self, expanded: bool, cx: &mut gpui_kit::App) {
        self.update(cx, |panel, cx| panel.set_expanded(expanded, cx));
    }
    fn set_docked_left(&self, left: bool, cx: &mut gpui_kit::App) {
        let side = if left { Side::Left } else { Side::Right };
        self.update(cx, |panel, cx| panel.set_side(side, cx));
    }
}

#[cfg(test)]
#[path = "registration_tests.rs"]
mod tests;
