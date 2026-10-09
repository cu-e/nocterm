//! The compact summary at the start of the footer, which opens the details.

use gpui_kit::{
    Anchor, AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Hsla, Window,
    component::{
        ActiveTheme as _, Icon, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        popover::Popover,
    },
    div,
    prelude::*,
    px,
};
use nocterm_monitor::{MonitorMetric, Snapshot, format};
use nocterm_ui::{IconName, SettingsExt as _};

use crate::{
    details::DetailsView,
    graph,
    model::{HostMonitor, Status},
};

pub(crate) struct MonitorStatus {
    monitor: Entity<HostMonitor>,
    details: Entity<DetailsView>,
    focus: FocusHandle,
}

impl MonitorStatus {
    pub(crate) fn new(monitor: Entity<HostMonitor>, cx: &mut Context<Self>) -> Self {
        cx.observe(&monitor, |_, _, cx| cx.notify()).detach();
        let details = cx.new(|cx| DetailsView::new(monitor.clone(), cx));
        Self {
            monitor,
            details,
            focus: cx.focus_handle(),
        }
    }
}

impl Render for MonitorStatus {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let monitor = self.monitor.read(cx);
        if *monitor.status() == Status::Off {
            return div().into_any_element();
        }
        let tooltip = match monitor.status() {
            Status::Offline => "Disconnected".to_owned(),
            Status::Detecting | Status::Starting => "Connecting to the host monitor…".to_owned(),
            Status::Unsupported(name) => format!("Resource monitoring is not available on {name}"),
            Status::Failed(error) => error.clone(),
            Status::Running | Status::Off => "Host resources".to_owned(),
        };
        let tooltip = match monitor.host() {
            Some(host) => format!("{} — {tooltip}", host.label()),
            None => tooltip,
        };
        let stale = !matches!(monitor.status(), Status::Running);
        let snapshot = monitor.sampler().map(|sampler| sampler.snapshot());
        let mut button = Button::new("host-monitor")
            .ghost()
            .small()
            .tooltip(tooltip)
            .child(summary(snapshot, stale, cx));
        if snapshot.is_none() {
            button = button.icon(IconName::Activity);
        }
        let open = monitor.is_open();
        let details = self.details.clone();
        let model = self.monitor.downgrade();
        Popover::new("host-monitor-details")
            .anchor(Anchor::BottomLeft)
            .trigger(button)
            .track_focus(&self.focus)
            .open(open)
            .on_open_change(move |open, _, cx| {
                let _ = model.update(cx, |monitor, cx| monitor.set_open(*open, cx));
            })
            .content(move |_, _, _| details.clone())
            .into_any_element()
    }
}

/// The configured metrics, side by side.
fn summary(snapshot: Option<&Snapshot>, stale: bool, cx: &App) -> AnyElement {
    let Some(snapshot) = snapshot else {
        return div().into_any_element();
    };
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    h_flex()
        .gap_3()
        .text_xs()
        .when(stale, |row| row.opacity(0.6))
        .children(
            cx.setting::<nocterm_settings::MonitorSettings>()
                .status_bar
                .iter()
                .map(|metric| item(*metric, snapshot, muted, cx)),
        )
        .into_any_element()
}

fn item(metric: MonitorMetric, snapshot: &Snapshot, muted: Hsla, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let icon = |name: IconName| Icon::new(name).xsmall().text_color(muted);
    let text = |label: &'static str| div().text_color(muted).child(label);
    let value = |value: Option<String>| div().child(value.unwrap_or_else(|| "—".into()));
    let with_meter = |name: IconName, percent: Option<f32>, color: Hsla| {
        h_flex()
            .gap_1()
            .child(icon(name))
            .child(value(percent.map(format::percent)))
            .when_some(percent, |row, percent| {
                row.child(div().w(px(18.)).child(graph::meter(percent, color, cx)))
            })
    };
    match metric {
        MonitorMetric::Cpu => with_meter(IconName::Cpu, snapshot.cpu, theme.chart_1),
        MonitorMetric::Memory => with_meter(
            IconName::MemoryStick,
            snapshot.memory.map(|memory| memory.percent()),
            theme.chart_2,
        ),
        MonitorMetric::Cores => h_flex()
            .gap_1()
            .child(icon(IconName::Cpu))
            .child(match &snapshot.cores {
                Some(cores) => graph::columns(cores, theme.chart_1, cx).into_any_element(),
                None => value(None).into_any_element(),
            }),
        MonitorMetric::Swap => h_flex().gap_1().child(text("Swap")).child(value(
            snapshot.swap.map(|swap| format::percent(swap.percent())),
        )),
        MonitorMetric::Load => h_flex()
            .gap_1()
            .child(text("Load"))
            .child(value(snapshot.load.map(|load| format!("{:.2}", load[0])))),
        MonitorMetric::Uptime => h_flex()
            .gap_1()
            .child(icon(IconName::Clock))
            .child(value(snapshot.uptime.map(format::duration))),
        MonitorMetric::Temperature => {
            h_flex()
                .gap_1()
                .child(icon(IconName::Thermometer))
                .child(value(
                    snapshot
                        .hottest()
                        .map(|sensor| format::celsius(sensor.celsius)),
                ))
        }
        MonitorMetric::Disks => with_meter(
            IconName::HardDrive,
            snapshot.fullest().map(|disk| disk.percent()),
            theme.chart_3,
        ),
        MonitorMetric::Network => h_flex()
            .gap_1()
            .child(icon(IconName::ArrowDown))
            .child(value(
                snapshot.network.map(|net| format::rate(net.received)),
            ))
            .child(icon(IconName::ArrowUp))
            .child(value(snapshot.network.map(|net| format::rate(net.sent)))),
        MonitorMetric::DiskIo => h_flex()
            .gap_1()
            .child(icon(IconName::HardDrive))
            .child(text("R"))
            .child(value(snapshot.disk_io.map(|disk| format::rate(disk.read))))
            .child(text("W"))
            .child(value(
                snapshot.disk_io.map(|disk| format::rate(disk.written)),
            )),
    }
    .into_any_element()
}
