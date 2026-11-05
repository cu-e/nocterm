//! One nearest-deadline timer for the visible contributions of one physical window.
use super::content::Content;
use gpui_kit::{App, AppContext as _, Context, Entity, Subscription, Task, WeakEntity, WindowId};
use nocterm_plugin_host::PluginHost;
use std::time::Instant;

pub(in crate::shell_setup) struct RefreshDriver {
    contents: Vec<WeakEntity<Content>>,
    timer: Option<Task<()>>,
    host: Entity<PluginHost>,
    window: WindowId,
    visible: bool,
    closed: bool,
    scheduled: bool,
    _subscription: Subscription,
}
impl RefreshDriver {
    pub(in crate::shell_setup) fn new(
        host: Entity<PluginHost>,
        window: WindowId,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| {
            let subscription = cx.observe(&host, |this: &mut Self, _, cx| this.wake(cx));
            cx.on_release(|this: &mut Self, cx| this.close(cx)).detach();
            Self {
                contents: Vec::new(),
                timer: None,
                host,
                window,
                visible: true,
                closed: false,
                scheduled: false,
                _subscription: subscription,
            }
        })
    }
    pub(super) fn add(&mut self, content: WeakEntity<Content>, cx: &mut Context<Self>) {
        self.contents.push(content);
        self.defer_wake(cx);
    }
    pub(super) fn defer_wake(&mut self, cx: &mut Context<Self>) {
        if self.scheduled || self.closed {
            return;
        }
        self.scheduled = true;
        let driver = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = driver.update(cx, |this, cx| {
                this.scheduled = false;
                this.wake(cx);
            });
        });
    }
    pub(super) fn visibility(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible != visible {
            self.visible = visible;
            self.wake(cx);
        }
    }
    pub(super) fn close(&mut self, cx: &mut App) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.timer.take();
        self.contents.clear();
        self.host
            .update(cx, |host, _| host.release_window(self.window));
    }
    pub(super) fn wake(&mut self, cx: &mut Context<Self>) {
        self.timer.take();
        if self.closed || !self.visible {
            return;
        }
        let now = cx.background_executor().now();
        let mut deadline: Option<Instant> = None;
        self.contents.retain(|content| {
            let Some(content) = content.upgrade() else {
                return false;
            };
            let next = content.update(cx, |content, cx| content.tick(now, cx));
            if let Some(next) = next {
                deadline = Some(deadline.map_or(next, |current| current.min(next)));
            }
            true
        });
        if let Some(deadline) = deadline {
            let wait = deadline.saturating_duration_since(cx.background_executor().now());
            self.timer = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(wait).await;
                let _ = this.update(cx, |this, cx| this.wake(cx));
            }));
        }
    }
}
