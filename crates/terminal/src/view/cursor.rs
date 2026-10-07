//! Cursor timing reuses one task across input bursts.
use super::*;
impl TerminalView {
    // ── Cursor ───────────────────────────────────────────────────────────────

    pub(super) fn blink(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let mut wait = CURSOR_BLINK_INTERVAL;
            loop {
                cx.background_executor().timer(wait).await;
                let next = this.update(cx, |this, cx| {
                    let now = cx.background_executor().now();
                    if now < this.next_blink {
                        return this.next_blink - now;
                    }
                    this.next_blink = now + CURSOR_BLINK_INTERVAL;
                    let blinking = this.frame.borrow().cursor.is_some_and(|c| c.blinking);
                    if this.focused && blinking {
                        this.cursor_lit = !this.cursor_lit;
                        cx.notify();
                    }
                    CURSOR_BLINK_INTERVAL
                });
                let Ok(next) = next else { break };
                wait = next;
            }
        })
    }

    /// Lights the cursor and restarts its blink, as after typing.
    pub(super) fn wake_cursor(&mut self, cx: &mut Context<Self>) {
        self.next_blink = cx.background_executor().now() + CURSOR_BLINK_INTERVAL;
        if !self.cursor_lit {
            self.cursor_lit = true;
            cx.notify();
        }
    }
}
