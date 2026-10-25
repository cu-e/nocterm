//! The registrations of a window follow the plugin set: a contribution of an installed plugin
//! is registered once, withdrawn when its plugin goes or changes, and created only while its
//! plugin is enabled.
use std::{cell::RefCell, collections::HashMap, rc::Rc};

use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, Subscription, WeakEntity, Window, WindowId,
};
use nocterm_plugin_api::{ContributionContext, Kind, LocalId, Zone};
use nocterm_plugin_host::{Call, PluginHost};
use nocterm_plugin_model::{
    ContributionId, ContributionKind, PluginId, UserContribution, user_label,
};
use nocterm_plugin_ports::PluginProviders;
use nocterm_plugin_view::FrameKind;
use nocterm_ui::{PluginSetHandle, PluginSource as _, contribution_icon};
use nocterm_workspace::{PanelRegistration, ShellCapabilities, WidgetRegistration, Workspace};

use super::{
    Badges,
    content::Content,
    views::{UserPluginPanel, UserPluginWidget},
};

/// What a registration was made from, to notice a plugin that changed.
#[derive(PartialEq)]
struct Registered {
    plugin: String,
    contribution: UserContribution,
}

/// Everything a contribution needs to be created, again and again.
#[derive(Clone)]
struct Parts {
    host: gpui_kit::Entity<PluginHost>,
    providers: PluginProviders,
    badges: Badges,
    dev: bool,
    call: Call,
    plugin: String,
    contribution: UserContribution,
}

impl Parts {
    fn content<T: 'static>(&self, kind: FrameKind, cx: &mut Context<T>) -> Content {
        Content::new(
            self.host.clone(),
            self.call.clone(),
            &self.plugin,
            kind,
            self.providers.clone(),
            self.dev,
            cx,
        )
    }
}

/// The user contributions registered in one window.
pub(in crate::shell_setup) struct WindowSync {
    host: gpui_kit::Entity<PluginHost>,
    source: PluginSetHandle,
    providers: PluginProviders,
    badges: Badges,
    /// Whether the frames are marked as a developer session's.
    dev: bool,
    window: WindowId,
    registered: HashMap<ContributionId, Registered>,
}

impl WindowSync {
    pub(in crate::shell_setup) fn new(
        host: gpui_kit::Entity<PluginHost>,
        source: PluginSetHandle,
        providers: PluginProviders,
        badges: Badges,
        dev: bool,
        window: WindowId,
    ) -> Self {
        Self {
            host,
            source,
            providers,
            badges,
            dev,
            window,
            registered: HashMap::new(),
        }
    }

    /// The set this window follows.
    pub(super) fn source(&self) -> &PluginSetHandle {
        &self.source
    }

    /// Withdraws what the set no longer holds or holds differently, then registers the rest.
    pub(super) fn run(
        &mut self,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        let set = self.source.current(cx);
        let mut wanted = Vec::new();
        for entry in set.users() {
            for contribution in entry.plugin.contributions() {
                let made = Registered {
                    plugin: entry.plugin.name().to_owned(),
                    contribution: contribution.clone(),
                };
                wanted.push((entry.plugin.id().clone(), made));
            }
        }
        self.registered.retain(|id, known| {
            let kept = wanted
                .iter()
                .any(|(_, made)| made.contribution.id() == id && made == known);
            if !kept && let Err(error) = ShellCapabilities::withdraw(workspace, id, window, cx) {
                tracing::warn!(%id, %error, "a user contribution was not withdrawn");
            }
            kept
        });
        for (plugin, made) in wanted {
            let id = made.contribution.id().clone();
            if self.registered.contains_key(&id) {
                continue;
            }
            let Some(call) = call_of(self.window, &plugin, &made.contribution) else {
                tracing::warn!(%id, "a user contribution has no usable local name");
                continue;
            };
            match self.register(call, &made, workspace, window, cx) {
                Ok(()) => {
                    self.registered.insert(id, made);
                }
                Err(error) => tracing::warn!(%error, %id, "a user contribution did not register"),
            }
        }
    }

    fn register(
        &self,
        call: Call,
        made: &Registered,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Result<(), nocterm_workspace::RegistrationError> {
        let contribution = &made.contribution;
        let parts = Parts {
            host: self.host.clone(),
            providers: self.providers.clone(),
            badges: self.badges.clone(),
            dev: self.dev,
            call,
            plugin: made.plugin.clone(),
            contribution: contribution.clone(),
        };
        match contribution.kind() {
            ContributionKind::Panel => workspace.register_panel(
                PanelRegistration::user(contribution.id(), move |_, _, cx| panel_of(&parts, cx)),
                window,
                cx,
            ),
            ContributionKind::Widget => workspace.register_widget(
                WidgetRegistration::user(contribution.id(), move |_, _, cx| widget_of(&parts, cx)),
                window,
                cx,
            ),
        }
    }
}

fn panel_of(parts: &Parts, cx: &mut Context<Workspace>) -> gpui_kit::Entity<UserPluginPanel> {
    let id = parts.contribution.id().clone();
    let workspace = cx.weak_entity();
    let panel = cx.new(|cx| {
        let content = parts.content(FrameKind::Panel, cx);
        UserPluginPanel::new(
            content,
            id.clone(),
            parts.contribution.title().to_owned().into(),
            contribution_icon(parts.contribution.icon()),
            workspace,
            cx,
        )
    });
    parts.badges.add(id, panel.downgrade());
    panel
}

fn widget_of(parts: &Parts, cx: &mut Context<Workspace>) -> gpui_kit::Entity<UserPluginWidget> {
    cx.new(|cx| {
        let content = parts.content(FrameKind::Bar, cx);
        let label = widget_label(&parts.plugin, parts.contribution.title(), parts.dev);
        UserPluginWidget::new(content, label.into(), cx)
    })
}

/// What the tooltip of a bar widget says: the plugin, then the widget, and DEV in a developer
/// session, whose bar item has no header line to say it.
fn widget_label(plugin: &str, title: &str, dev: bool) -> String {
    let label = user_label(plugin, title);
    if dev {
        format!("{label} \u{b7} DEV")
    } else {
        label
    }
}

/// The call for the contribution's views in a window: its local name and where it is drawn.
/// `None` when the contribution's id holds no local name the host accepts.
fn call_of(window: WindowId, plugin: &PluginId, contribution: &UserContribution) -> Option<Call> {
    let local = contribution
        .id()
        .as_str()
        .split_once('.')
        .map_or("", |(_, local)| local);
    let placement = contribution.placement();
    Some(Call {
        window,
        plugin: plugin.clone(),
        local: LocalId::parse(local).ok()?,
        context: ContributionContext {
            kind: match contribution.kind() {
                ContributionKind::Panel => Kind::Panel,
                ContributionKind::Widget => Kind::Widget,
            },
            zone: match placement.zone {
                nocterm_plugin_model::Zone::Left => Zone::Left,
                nocterm_plugin_model::Zone::Center => Zone::Center,
                nocterm_plugin_model::Zone::Right => Zone::Right,
            },
        },
        in_focus: false,
    })
}

/// Runs the sync whenever the set is published, for as long as the subscription lives.
pub(super) fn follow(
    sync: Rc<RefCell<WindowSync>>,
    source: &PluginSetHandle,
    workspace: WeakEntity<Workspace>,
    window: AnyWindowHandle,
    cx: &mut gpui_kit::App,
) -> Subscription {
    source.observe(
        Box::new(move |cx| {
            // A window that is gone has nothing left to register.
            let _ = window.update(cx, |_, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    sync.borrow_mut().run(workspace, window, cx);
                })
            });
        }),
        cx,
    )
}

#[cfg(test)]
mod tests {
    use super::widget_label;

    #[test]
    fn the_tooltip_of_a_developer_widget_ends_in_dev_and_an_installed_one_does_not() {
        assert_eq!(
            widget_label("Example Clock", "Clock", false),
            "Plugin \u{b7} Example Clock \u{2014} Clock"
        );
        assert_eq!(
            widget_label("Example Clock", "Clock", true),
            "Plugin \u{b7} Example Clock \u{2014} Clock \u{b7} DEV"
        );
    }
}
