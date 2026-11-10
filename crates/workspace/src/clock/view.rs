//! The status bar widget that shows the time.
use std::{sync::Arc, time::Duration};

use gpui_kit::{
    Context, IntoElement, Render, Subscription, Task, TestSupportExt as _, Window,
    component::ActiveTheme as _, div, prelude::*,
};
use nocterm_core::clock::WallClock;
use nocterm_store::SettingsHandle;

use super::settings::ClockSettings;

/// The local time, written as `[clock]` says. It re-reads the clock when the text can change
/// (each second with seconds on, else each minute boundary, never later than five seconds) for
/// as long as it lives, and follows a change of the settings at once. Dropping the view drops
/// the timer.
pub(crate) struct ClockView {
    clock: Arc<dyn WallClock>,
    settings: SettingsHandle,
    style: ClockSettings,
    shown: String,
    timer: Task<()>,
    _follow: Subscription,
}

impl ClockView {
    /// The `[clock]` section must be registered with `settings`.
    pub(crate) fn new(
        clock: Arc<dyn WallClock>,
        settings: SettingsHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        let style = settings.setting::<ClockSettings>(cx).clone();
        let shown = style.render(clock.now());
        let this = cx.weak_entity();
        let follow = settings.observe(cx, move |cx| {
            let _ = this.update(cx, |view, cx| view.restyle(cx));
        });
        Self {
            shown,
            timer: Self::arm(cx),
            clock,
            settings,
            style,
            _follow: follow,
        }
    }

    /// A timer that reads the clock now and then once the text can have changed. The task ends
    /// with the view, before any more reads.
    fn arm(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            // `update` fails once the view is gone, which ends the loop.
            while let Ok(wait) = this.update(cx, |view, cx| view.read_clock(cx)) {
                cx.background_executor().timer(wait).await;
            }
        })
    }

    /// Reads the clock, shows the time, and says how long until the text can change.
    fn read_clock(&mut self, cx: &mut Context<Self>) -> Duration {
        let time = self.clock.now();
        let shown = self.style.render(time);
        if self.shown != shown {
            self.shown = shown;
            cx.notify();
        }
        self.style.wait(time)
    }

    /// Follows the settings: a changed style shows at once and the timer takes the cadence of
    /// the new style.
    fn restyle(&mut self, cx: &mut Context<Self>) {
        let style = self.settings.setting::<ClockSettings>(cx);
        if *style == self.style {
            return;
        }
        self.style = style.clone();
        // The new timer reads first, so the text is right before the old cadence would show it.
        self.timer = Self::arm(cx);
    }

    /// What the view shows.
    #[cfg(test)]
    pub(crate) fn text(&self) -> String {
        self.shown.clone()
    }
}

impl Render for ClockView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("status-clock")
            .test_support()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(self.shown.clone())
    }
}
