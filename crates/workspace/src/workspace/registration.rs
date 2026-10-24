//! Panels and widgets registered under the ids of the plugin catalog.
//!
//! An owner names its contribution and hands the workspace a factory. The workspace checks the
//! id against the catalog its [`Deps`](super::Deps) carry, creates the view and places it
//! according to the descriptor. This module holds free items only; the `Workspace` methods that
//! apply a registration live beside the other composition methods.

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

use gpui_kit::{Context, Entity, EventEmitter, Pixels, Render, Subscription, Window};
use nocterm_plugin_model::{
    ContributionId, ContributionKind, Owner, PanelChrome, PanelTraits, PluginCatalog, Side,
    SystemContribution,
};

use super::{
    Workspace,
    chrome::WidgetFactory,
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
    /// The workspace calls `factory` once, at registration, and borrows itself to it: the
    /// factory may read the workspace but cannot update it.
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

    fn of(contribution: &SystemContribution, origin: Origin<T>) -> Self {
        Self {
            id: contribution.id.clone(),
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

/// A status bar widget to register for a contribution.
pub struct WidgetRegistration {
    id: ContributionId,
    create: WidgetFactory,
}

impl WidgetRegistration {
    /// The workspace calls `factory` when the window's arrangement places the widget, and
    /// borrows itself to it: the factory may read the workspace but cannot update it. A widget
    /// that the arrangement leaves out is not created.
    pub fn new<V: Render>(
        contribution: &SystemContribution,
        factory: impl Fn(&Workspace, &mut Window, &mut Context<Workspace>) -> Entity<V> + 'static,
    ) -> Self {
        Self {
            id: contribution.id.clone(),
            create: Rc::new(move |workspace, window, cx| factory(workspace, window, cx).into()),
        }
    }

    pub(super) fn id(&self) -> &ContributionId {
        &self.id
    }
    pub(super) fn into_factory(self) -> WidgetFactory {
        self.create
    }
}

/// The catalog a window was composed with and the contributions registered against it.
pub(super) struct Registrations {
    catalog: Arc<PluginCatalog>,
    registered: HashSet<ContributionId>,
    /// How to create the panel of a registration again, by contribution id.
    factories: HashMap<ContributionId, PanelFactory>,
}

impl Registrations {
    pub(super) fn new(catalog: Arc<PluginCatalog>) -> Self {
        Self {
            catalog,
            registered: HashSet::new(),
            factories: HashMap::new(),
        }
    }

    /// Keeps the factory of a registered panel.
    pub(super) fn keep_factory(&mut self, id: ContributionId, factory: PanelFactory) {
        self.factories.insert(id, factory);
    }

    pub(super) fn catalog(&self) -> &PluginCatalog {
        &self.catalog
    }

    pub(super) fn is_registered(&self, id: &ContributionId) -> bool {
        self.registered.contains(id)
    }

    /// The catalog's descriptor of a contribution, if something was registered for it.
    pub(super) fn registered_contribution(
        &self,
        id: &ContributionId,
    ) -> Option<&'static SystemContribution> {
        self.is_registered(id)
            .then(|| self.catalog.contribution(id))
            .flatten()
    }

    /// The id of the catalog's contribution with this text, for commands that carry one.
    pub(super) fn named(&self, name: &str) -> Option<ContributionId> {
        self.catalog
            .entries()
            .find(|contribution| contribution.id.as_str() == name)
            .map(|contribution| contribution.id.clone())
    }

    /// Checks a panel registration and records it. `expandable` and `eager` describe how the
    /// owner wants to host the panel.
    pub(super) fn admit_panel(
        &mut self,
        id: &ContributionId,
        expandable: bool,
        eager: bool,
    ) -> Result<(&'static SystemContribution, PanelTraits), RegistrationError> {
        let contribution = self.check(id, ContributionKind::Panel)?;
        // The catalog refuses a panel without traits, so this guard only names the kind.
        let Some(traits) = contribution.panel else {
            return Err(RegistrationError::KindMismatch {
                id: id.clone(),
                expected: ContributionKind::Widget,
            });
        };
        if expandable && traits.chrome != PanelChrome::OwnHeader {
            return Err(RegistrationError::NotExpandable(id.clone()));
        }
        let may_be_disabled = matches!(
            self.catalog.owner(id),
            Some(Owner::Plugin(plugin)) if !plugin.required
        );
        if eager && may_be_disabled {
            return Err(RegistrationError::EagerNotRequired(id.clone()));
        }
        self.registered.insert(id.clone());
        Ok((contribution, traits))
    }

    /// Checks a widget registration and records it.
    pub(super) fn admit_widget(
        &mut self,
        id: &ContributionId,
    ) -> Result<&'static SystemContribution, RegistrationError> {
        let contribution = self.check(id, ContributionKind::Widget)?;
        self.registered.insert(id.clone());
        Ok(contribution)
    }

    fn check(
        &self,
        id: &ContributionId,
        kind: ContributionKind,
    ) -> Result<&'static SystemContribution, RegistrationError> {
        let contribution = self
            .catalog
            .contribution(id)
            .ok_or_else(|| RegistrationError::Unlisted(id.clone()))?;
        if contribution.kind != kind {
            return Err(RegistrationError::KindMismatch {
                id: id.clone(),
                expected: contribution.kind,
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
