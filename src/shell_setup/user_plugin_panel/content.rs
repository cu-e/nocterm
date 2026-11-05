//! What a panel and a widget share: the plugin's view, drawn in its frame, and the calls that
//! refresh it and report what the user did to it.
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use super::driver::RefreshDriver;
use gpui_kit::{
    App, AppContext as _, Entity, FocusHandle, IntoElement as _, WeakEntity,
    component::{ActiveTheme as _, Icon, Sizable as _},
};
use nocterm_plugin_api::{Event, PluginIcon, Refresh, View};
use nocterm_plugin_host::{Call, CallError, Input, PluginHost};
use nocterm_plugin_model::ContributionId;
use nocterm_plugin_ports::PluginProviders;
use nocterm_plugin_view::{FrameKind, Hooks, Interaction, Origin, PluginView, ViewStyle};
use nocterm_ui::contribution_icon;
use nocterm_workspace::Workspace;

/// The shortest wait between two draws the shell grants.
const MIN_WAIT: Duration = Duration::from_millis(250);
/// How long a window waits before it looks again at a view that asked for no refresh.
const IDLE: Duration = Duration::from_secs(1);

pub(super) struct FrameSpec<'a> {
    pub(super) name: &'a str,
    pub(super) kind: FrameKind,
    pub(super) dev: bool,
}

/// The plugin's view and the way to ask the host for the next one.
pub(super) struct Content {
    host: Entity<PluginHost>,
    view: Entity<PluginView>,
    call: Call,
    providers: PluginProviders,
    revision: u64,
    next: Instant,
    visible: bool,
    last: Option<View>,
    appearance: ViewStyle,
    panel: Option<(WeakEntity<Workspace>, ContributionId)>,
    driver: WeakEntity<RefreshDriver>,
}

impl Content {
    pub(super) fn new<T: 'static>(
        host: Entity<PluginHost>,
        driver: Entity<RefreshDriver>,
        call: Call,
        frame: FrameSpec<'_>,
        providers: PluginProviders,
        cx: &mut gpui_kit::Context<T>,
    ) -> Entity<Self> {
        let hooks = hooks(host.clone(), call.clone(), providers.clone());
        let view = cx.new(|cx| {
            let mut view = PluginView::new(frame.name.to_owned(), frame.kind, style(cx), hooks);
            if frame.dev {
                view.mark_dev();
            }
            view
        });
        let revision = host.read(cx).revision();
        let now = cx.background_executor().now();
        let appearance = style(cx);
        let entity = cx.new(|_| Self {
            host,
            view,
            call,
            providers,
            revision,
            next: now,
            visible: false,
            last: None,
            appearance,
            panel: None,
            driver: driver.downgrade(),
        });
        driver.update(cx, |driver, cx| driver.add(entity.downgrade(), cx));
        entity
    }

    pub(super) fn view(&self) -> &Entity<PluginView> {
        &self.view
    }

    pub(super) fn panel(&mut self, workspace: WeakEntity<Workspace>, id: ContributionId) {
        self.panel = Some((workspace, id));
    }
    pub(super) fn visible(&mut self, visible: bool, in_focus: bool, cx: &mut App) {
        let changed = self.visible != visible || self.call.in_focus != in_focus;
        self.visible = visible;
        self.call.in_focus = in_focus;
        if changed {
            self.next = cx.background_executor().now();
            let driver = self.driver.clone();
            cx.defer(move |cx| {
                let _ = driver.update(cx, |driver, cx| driver.defer_wake(cx));
            });
        }
        self.restyle(cx);
    }
    pub(super) fn restyle(&mut self, cx: &mut App) {
        let next = style(cx);
        if style_key(next) != style_key(self.appearance) {
            self.appearance = next;
            self.view.update(cx, |view, cx| {
                view.restyle(next);
                cx.notify();
            });
        }
    }
    pub(super) fn tick(&mut self, now: Instant, cx: &mut App) -> Option<Instant> {
        if let Some((workspace, id)) = &self.panel {
            self.visible = workspace
                .read_with(cx, |workspace, _| workspace.panel_is_open(id))
                .unwrap_or(false);
        }
        if !self.visible {
            return None;
        }
        if now >= self.next || self.host.read(cx).revision() != self.revision {
            self.refresh(now, cx);
        }
        Some(self.next)
    }

    /// Asks the host for the view and shows it. A plugin that failed shows the fault chip; one
    /// over the render budget keeps its last view and is asked again soon.
    fn refresh(&mut self, now: Instant, cx: &mut App) {
        let answer = self
            .host
            .update(cx, |host, cx| host.render(&self.call, &self.providers, cx));
        self.revision = self.host.read(cx).revision();
        self.restyle(cx);
        match answer {
            Ok(view) => {
                self.next = now + wait_of(view.refresh);
                let changed = self
                    .last
                    .as_ref()
                    .is_none_or(|last| last.nodes != view.nodes);
                if changed {
                    let shown = view.clone();
                    self.view.update(cx, |plugin_view, cx| {
                        if let Err(error) = plugin_view.show(shown) { tracing::warn!(%error, plugin = %self.call.plugin, "a plugin view was refused"); }
                        cx.notify();
                    });
                }
                self.last = Some(view);
            }
            Err(CallError::Throttled { retry_after_ms }) => {
                self.next = now + Duration::from_millis(u64::from(retry_after_ms)).max(MIN_WAIT);
            }
            Err(CallError::NotRunning(_))
                if self.host.read(cx).plugins().iter().any(|plugin| {
                    plugin.id == self.call.plugin
                        && matches!(plugin.status, nocterm_plugin_host::PluginStatus::Running)
                }) =>
            {
                // Native data publication temporarily holds effects; keep the last valid tree.
                self.next = now + MIN_WAIT;
            }
            Err(error) => {
                tracing::debug!(%error, plugin = %self.call.plugin, "a plugin view is not drawn");
                self.next = now + IDLE;
                self.last = None;
                self.view.update(cx, |plugin_view, cx| {
                    if !plugin_view.is_fault() {
                        plugin_view.fault();
                        cx.notify();
                    }
                });
            }
        }
    }
}

fn style_key(style: ViewStyle) -> [gpui_kit::Hsla; 7] {
    [
        style.text,
        style.muted,
        style.success,
        style.caution,
        style.border,
        style.hover,
        style.marker,
    ]
}

fn wait_of(refresh: Option<Refresh>) -> Duration {
    match refresh {
        None | Some(Refresh::NextSecond) => IDLE,
        Some(Refresh::NextMinute) => Duration::from_secs(60),
        Some(Refresh::AfterMs(ms)) => Duration::from_millis(u64::from(ms)).max(MIN_WAIT),
    }
}

/// The colours of the frame and of the view, all from the theme, so a theme author controls
/// them. The caution tone is the theme's warning colour; the frame's marker and label, not the
/// colour, tell a plugin from the host.
fn style(cx: &App) -> ViewStyle {
    let theme = cx.theme();
    ViewStyle {
        text: theme.foreground,
        muted: theme.muted_foreground,
        success: theme.success,
        caution: theme.warning,
        border: theme.border,
        hover: theme.accent,
        marker: theme.info,
    }
}

/// What the view needs of the application: icons, events, and the focus handles of its controls.
fn hooks(host: Entity<PluginHost>, call: Call, providers: PluginProviders) -> Hooks {
    let handles: Rc<RefCell<Vec<FocusHandle>>> = Rc::default();
    Hooks {
        icon: Rc::new(|icon: PluginIcon| {
            Icon::new(contribution_icon(icon.to_icon()))
                .small()
                .into_any_element()
        }),
        event: Rc::new(move |interaction: Interaction, _, cx: &mut App| {
            let input = match interaction.origin {
                Origin::Pointer => Input::Pointer,
                Origin::Key => Input::Key { repeat: false },
            };
            report(&host, &call, input, interaction.event, &providers, cx);
        }),
        handles: Rc::new(move |slot, cx: &mut App| {
            let mut handles = handles.borrow_mut();
            while handles.len() <= slot {
                handles.push(cx.focus_handle().tab_stop(true));
            }
            handles[slot].clone()
        }),
    }
}

fn report(
    host: &Entity<PluginHost>,
    call: &Call,
    input: Input,
    event: Event,
    providers: &PluginProviders,
    cx: &mut App,
) {
    let call = Call {
        in_focus: true,
        ..call.clone()
    };
    let handled = host.update(cx, |host, cx| {
        host.handle(&call, input, event, providers, cx)
    });
    if let Err(error) = handled {
        tracing::debug!(%error, plugin = %call.plugin, "a plugin did not take an event");
    }
}
