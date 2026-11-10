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
    component::{ActiveTheme as _, Icon, tooltip::Tooltip},
};
use nocterm_plugin_api::{Event, Refresh, View};
use nocterm_plugin_host::{Call, CallError, Input, PluginHost};
use nocterm_plugin_model::ContributionId;
use nocterm_plugin_ports::PluginProviders;
use nocterm_plugin_view::{FrameKind, Hooks, Interaction, Origin, PluginView, ViewStyle};
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
    /// The visibility the plugin was last told, by a `visibility-changed` event.
    told: Option<bool>,
    last: Option<View>,
    appearance: ViewStyle,
    panel: Option<(WeakEntity<Workspace>, ContributionId)>,
    driver: WeakEntity<RefreshDriver>,
}

impl Content {
    pub(super) fn new(
        host: Entity<PluginHost>,
        driver: Entity<RefreshDriver>,
        call: Call,
        frame: FrameSpec<'_>,
        providers: PluginProviders,
        cx: &mut App,
    ) -> Entity<Self> {
        let mono = cx.theme().mono_font_family.clone();
        let shown: Rc<RefCell<Option<WeakEntity<PluginView>>>> = Rc::default();
        let hooks = hooks(
            host.clone(),
            call.clone(),
            providers.clone(),
            mono,
            shown.clone(),
        );
        let view = cx.new(|cx| {
            let mut view = PluginView::new(frame.name.to_owned(), frame.kind, style(cx), hooks);
            if frame.dev {
                view.mark_dev();
            }
            view
        });
        *shown.borrow_mut() = Some(view.downgrade());
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
            told: None,
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
        self.tell_visibility(cx);
        self.restyle(cx);
    }

    /// Tells the plugin when its contribution is shown or hidden, once per change, after the
    /// current update: a plugin that refreshes on its own starts and stops with what it shows.
    fn tell_visibility(&mut self, cx: &mut App) {
        if self.told == Some(self.visible) || (self.told.is_none() && !self.visible) {
            return;
        }
        self.told = Some(self.visible);
        let (host, call, providers) =
            (self.host.clone(), self.call.clone(), self.providers.clone());
        let event = Event::VisibilityChanged {
            visible: self.visible,
        };
        cx.defer(move |cx| {
            let told = host.update(cx, |host, cx| host.deliver(&call, None, event, &providers, cx));
            if let Err(error) = told {
                tracing::debug!(%error, plugin = %call.plugin, "a plugin was not told its visibility");
            }
        });
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
            self.tell_visibility(cx);
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

fn style_key(style: ViewStyle) -> [gpui_kit::Hsla; 10] {
    [
        style.text,
        style.muted,
        style.success,
        style.caution,
        style.border,
        style.hover,
        style.selected,
        style.marker,
        style.chart[0],
        style.chart[1],
    ]
}

fn wait_of(refresh: Option<Refresh>) -> Duration {
    match refresh {
        None | Some(Refresh::NextSecond) => IDLE,
        Some(Refresh::NextMinute) => Duration::from_secs(60),
        Some(Refresh::AfterMs(ms)) => Duration::from_millis(u64::from(ms)).max(MIN_WAIT),
    }
}

/// The colours of the frame and of the view, all from the theme and the same the application's
/// own panels use, so a theme author controls them. The caution tone is the theme's warning
/// colour; the frame, not the colour, tells a plugin from the host.
fn style(cx: &App) -> ViewStyle {
    let theme = cx.theme();
    ViewStyle {
        text: theme.sidebar_foreground,
        muted: theme.muted_foreground,
        success: theme.success,
        caution: theme.warning,
        border: theme.border,
        hover: theme.sidebar_accent,
        selected: theme.foreground.opacity(0.08),
        marker: theme.info,
        chart: [theme.chart_1, theme.chart_2],
    }
}

/// What the view needs of the application: icons, tooltips, events, the focus handles of its
/// controls, and a redraw of `shown` when the user folds one of its sections.
fn hooks(
    host: Entity<PluginHost>,
    call: Call,
    providers: PluginProviders,
    mono: gpui_kit::SharedString,
    shown: Rc<RefCell<Option<WeakEntity<PluginView>>>>,
) -> Hooks {
    let handles: Rc<RefCell<Vec<FocusHandle>>> = Rc::default();
    Hooks {
        icon: Rc::new(|glyph| Icon::new(super::glyphs::icon_name(glyph)).into_any_element()),
        tooltip: Rc::new(|label, window, cx| Tooltip::new(label).build(window, cx)),
        redraw: Rc::new(move |cx: &mut App| {
            let view = shown.borrow().clone();
            if let Some(view) = view.and_then(|view| view.upgrade()) {
                view.update(cx, |_, cx| cx.notify());
            }
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
        mono,
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
