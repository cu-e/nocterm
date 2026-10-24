//! The details popover opens toward the inside of the window from the zone that places it.
use super::*;
use crate::status::MonitorStatus;
use gpui_kit::{Bounds, Pixels, px, test::TestWindowExt as _};
use nocterm_plugin_model::Zone;
use nocterm_workspace::ZoneAware as _;

/// Shows the summary in the middle of the window, where either side has room.
struct Middle(Entity<MonitorStatus>);

impl Render for Middle {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().flex().pl(px(500.)).pt(px(300.)).child(self.0.clone())
    }
}

fn details_in(zone: Zone, cx: &mut TestAppContext) -> (Bounds<Pixels>, Bounds<Pixels>) {
    let (_, _, monitor) = fixture(cx, Some(FakeHost::new("Linux")));
    cx.run_until_parked();
    let (window, _) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            let status = cx.new(|cx| MonitorStatus::new(monitor.clone(), cx));
            status.update(cx, |status, cx| status.set_zone(zone, cx));
            cx.new(|_| Middle(status))
        })
        .unwrap()
    });
    monitor.update(cx, |monitor, cx| monitor.set_open(true, cx));
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        (
            window.find("host-monitor").bounds(),
            window.find("host-monitor-details").bounds(),
        )
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_left_zone_opens_the_details_rightwards_from_the_summary(cx: &mut TestAppContext) {
    let (summary, details) = details_in(Zone::Left, cx);
    assert!(
        (details.left() - summary.left()).abs() < px(16.),
        "{details:?} starts at {summary:?}"
    );
}

#[gpui_kit::test]
fn a_right_zone_opens_the_details_leftwards_from_the_summary(cx: &mut TestAppContext) {
    let (summary, details) = details_in(Zone::Right, cx);
    assert!(
        (details.right() - summary.right()).abs() < px(16.),
        "{details:?} ends at {summary:?}"
    );
}
