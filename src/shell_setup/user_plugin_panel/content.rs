//! What a panel and a widget share: the plugin's view, drawn in its frame, and the calls that
//! refresh it and report what the user did to it.
use std::{cell::RefCell, rc::Rc, time::Duration};

use gpui_kit::{
    App, AppContext as _, Entity, FocusHandle, IntoElement as _,
    component::{ActiveTheme as _, Icon, Sizable as _},
};
use nocterm_plugin_api::{Event, PluginIcon, Refresh};
use nocterm_plugin_host::{Call, CallError, Input, PluginHost};
use nocterm_plugin_ports::PluginProviders;
use nocterm_plugin_view::{FrameKind, Hooks, Interaction, Origin, PluginView, ViewStyle};
use nocterm_ui::contribution_icon;

/// How often a window checks whether its plugin views are due.
const POLL: Duration = Duration::from_millis(250);
/// The shortest wait between two draws the shell grants.
const MIN_WAIT: Duration = Duration::from_millis(250);
/// How long a window waits before it looks again at a view that asked for no refresh.
const IDLE: Duration = Duration::from_secs(1);

/// The plugin's view and the way to ask the host for the next one.
pub(super) struct Content {
    host: Entity<PluginHost>,
    view: Entity<PluginView>,
    call: Call,
    providers: PluginProviders,
    revision: u64,
    wait: Duration,
    waited: Duration,
}

impl Content {
    pub(super) fn new<T: 'static>(
        host: Entity<PluginHost>,
        call: Call,
        name: &str,
        kind: FrameKind,
        providers: PluginProviders,
        cx: &mut gpui_kit::Context<T>,
    ) -> Self {
        let hooks = hooks(host.clone(), call.clone(), providers.clone());
        let view = cx.new(|cx| PluginView::new(name.to_owned(), kind, style(cx), hooks));
        let revision = host.read(cx).revision();
        Self {
            host,
            view,
            call,
            providers,
            revision,
            wait: Duration::ZERO,
            waited: Duration::ZERO,
        }
    }

    pub(super) fn view(&self) -> &Entity<PluginView> {
        &self.view
    }

    /// One step of the refresh loop: asks the host for a new view when the plugin's own refresh
    /// is due or the host's records changed, but only while the view `shown`.
    pub(super) fn poll(&mut self, shown: bool, in_focus: bool, cx: &mut App) {
        self.waited += POLL;
        let stale = self.host.read(cx).revision() != self.revision;
        if shown && (stale || self.waited >= self.wait) {
            self.refresh(in_focus, cx);
        }
    }

    /// Asks the host for the view and shows it. A plugin that failed shows the fault chip; one
    /// over the render budget keeps its last view and is asked again soon.
    fn refresh(&mut self, in_focus: bool, cx: &mut App) {
        self.waited = Duration::ZERO;
        self.call.in_focus = in_focus;
        let answer = self
            .host
            .update(cx, |host, cx| host.render(&self.call, &self.providers, cx));
        self.revision = self.host.read(cx).revision();
        let style = style(cx);
        match answer {
            Ok(view) => {
                self.wait = wait_of(view.refresh);
                self.view.update(cx, |plugin_view, cx| {
                    plugin_view.restyle(style);
                    if let Err(error) = plugin_view.show(view) {
                        tracing::warn!(%error, plugin = %self.call.plugin, "a plugin view was refused");
                    }
                    cx.notify();
                });
            }
            Err(CallError::Throttled { retry_after_ms }) => {
                self.wait = Duration::from_millis(u64::from(retry_after_ms)).max(MIN_WAIT);
            }
            Err(error) => {
                tracing::debug!(%error, plugin = %self.call.plugin, "a plugin view is not drawn");
                self.wait = IDLE;
                self.view.update(cx, |plugin_view, cx| {
                    plugin_view.fault();
                    cx.notify();
                });
            }
        }
    }
}

/// Runs `tick` on the owner now and then every quarter second until the owner is gone.
pub(super) fn drive<T: 'static>(
    cx: &mut gpui_kit::Context<T>,
    tick: impl Fn(&mut T, &mut gpui_kit::Context<T>) + 'static,
) -> gpui_kit::Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            if this.update(cx, |this, cx| tick(this, cx)).is_err() {
                break;
            }
            cx.background_executor().timer(POLL).await;
        }
    })
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
