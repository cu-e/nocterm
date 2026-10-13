//! One cursor timer is retained across all input bursts.
use super::*;

pub(super) struct CursorClock {
    lit: bool,
    deadline: Instant,
    _task: Task<()>,
}
impl CursorClock {
    pub(super) fn new(cx: &mut Context<TerminalView>) -> Self {
        Self {
            lit: true,
            deadline: cx.background_executor().now() + CURSOR_BLINK_INTERVAL,
            _task: spawn(cx),
        }
    }
    pub(super) fn lit(&self) -> bool {
        self.lit
    }
    pub(super) fn wake(&mut self, now: Instant) -> bool {
        self.deadline = now + CURSOR_BLINK_INTERVAL;
        let redraw = !self.lit;
        self.lit = true;
        redraw
    }
    pub(super) fn tick(&mut self, now: Instant, focused: bool, blinking: bool) -> (Duration, bool) {
        if now < self.deadline {
            return (self.deadline - now, false);
        }
        self.deadline = now + CURSOR_BLINK_INTERVAL;
        let redraw = focused && blinking;
        if redraw {
            self.lit = !self.lit;
        }
        (CURSOR_BLINK_INTERVAL, redraw)
    }
}
fn spawn(cx: &mut Context<TerminalView>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        let mut wait = CURSOR_BLINK_INTERVAL;
        loop {
            cx.background_executor().timer(wait).await;
            let next = this.update(cx, |this, cx| this.tick_cursor(cx));
            let Ok(next) = next else { break };
            wait = next;
        }
    })
}
