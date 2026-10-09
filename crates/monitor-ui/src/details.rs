//! The popup with everything the details setting asks for.

use gpui_kit::{
    AnyElement, App, Context, Entity, Hsla, SharedString, Window,
    component::{
        ActiveTheme as _, Icon, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
    px, rems,
};
use nocterm_monitor::{History, MonitorMetric, Snapshot, format};
use nocterm_settings::MonitorSettings;
use nocterm_ui::{IconName, SettingsExt as _};

use crate::{
    OpenMonitorSettings,
    graph::{self, Line},
    model::{HostMonitor, Status},
};

pub(crate) struct DetailsView {
    monitor: Entity<HostMonitor>,
}

impl DetailsView {
    pub(crate) fn new(monitor: Entity<HostMonitor>, cx: &mut Context<Self>) -> Self {
        cx.observe(&monitor, |_, _, cx| cx.notify()).detach();
        Self { monitor }
    }
}

impl Render for DetailsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let monitor = self.monitor.read(cx);
        let settings = cx.setting::<nocterm_settings::MonitorSettings>();
        let shows = |metric| settings.details.contains(&metric);
        let mut body = v_flex().gap_3();
        let title = monitor
            .sampler()
            .and_then(|sampler| sampler.snapshot().host.as_ref())
            .map(|host| host.hostname.clone())
            .filter(|name| !name.is_empty())
            .or_else(|| monitor.host().map(|host| host.label()))
            .unwrap_or_default();
        body = body.child(header(title, monitor, settings, cx));
        match monitor.sampler() {
            Some(sampler) if has_readings(sampler.snapshot()) => {
                let (snapshot, history) = (sampler.snapshot(), sampler.history());
                let span = f64::from(settings.history_points * settings.detail_interval_secs);
                for metric in MonitorMetric::ALL
                    .into_iter()
                    .filter(|metric| shows(*metric))
                {
                    if let Some(section) = section(metric, snapshot, history, span, cx) {
                        body = body.child(section);
                    }
                }
            }
            _ => body = body.child(placeholder(monitor.status(), cx)),
        }
        v_flex()
            .id("host-monitor-details")
            .w(rems(26.))
            .max_h(rems(36.))
            .overflow_y_scroll()
            .gap_3()
            .child(body)
            .child(footer(settings, cx))
    }
}

fn header(
    title: String,
    monitor: &HostMonitor,
    settings: &MonitorSettings,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let snapshot = monitor.sampler().map(|sampler| sampler.snapshot());
    let host = snapshot.and_then(|snapshot| snapshot.host.as_ref());
    let system = host
        .map(|host| {
            [host.os.as_str(), host.kernel.as_str()]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · ")
        })
        .unwrap_or_default();
    let mut facts = Vec::new();
    if let Some(uptime) = snapshot.and_then(|snapshot| snapshot.uptime)
        && settings.details.contains(&MonitorMetric::Uptime)
    {
        facts.push(format!("up {}", format::duration(uptime)));
    }
    if let Some([one, five, fifteen]) = snapshot.and_then(|snapshot| snapshot.load)
        && settings.details.contains(&MonitorMetric::Load)
    {
        facts.push(format!("load {one:.2} {five:.2} {fifteen:.2}"));
    }
    let (state, color) = match monitor.status() {
        Status::Running => (
            format!("every {}s", settings.detail_interval_secs),
            theme.success,
        ),
        Status::Detecting | Status::Starting => ("connecting".into(), theme.muted_foreground),
        Status::Offline => ("disconnected".into(), theme.muted_foreground),
        Status::Failed(_) => ("retrying".into(), theme.warning),
        Status::Unsupported(_) => ("unsupported".into(), theme.muted_foreground),
        Status::Off => ("off".into(), theme.muted_foreground),
    };
    v_flex()
        .gap_0p5()
        .child(
            h_flex()
                .gap_2()
                .child(
                    Icon::new(IconName::Server)
                        .small()
                        .text_color(theme.muted_foreground),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_semibold()
                        .child(title),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(div().size(px(6.)).rounded_full().bg(color))
                        .child(state),
                ),
        )
        .when(!system.is_empty(), |header| {
            header.child(muted_line(system, cx))
        })
        .when_some(host.map(|host| host.cpu_model.clone()), |header, model| {
            header.when(!model.is_empty(), |header| {
                header.child(muted_line(model, cx))
            })
        })
        .when(!facts.is_empty(), |header| {
            header.child(muted_line(facts.join("  ·  "), cx))
        })
        .into_any_element()
}

/// Whether any frame arrived yet: every script reports the host or its uptime.
fn has_readings(snapshot: &Snapshot) -> bool {
    snapshot.host.is_some() || snapshot.uptime.is_some()
}

fn muted_line(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    div()
        .text_xs()
        .truncate()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

fn placeholder(status: &Status, cx: &App) -> AnyElement {
    let text = match status {
        Status::Offline => "The session is disconnected.".to_owned(),
        Status::Unsupported(name) => {
            format!("Resource monitoring is not available on {name}.")
        }
        Status::Failed(error) => format!("{error} Trying again shortly."),
        Status::Off => "Nothing to show. Choose what to show in the monitor settings.".into(),
        Status::Detecting | Status::Starting | Status::Running => "Collecting…".into(),
    };
    div()
        .py_4()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

fn footer(settings: &MonitorSettings, cx: &App) -> AnyElement {
    let theme = cx.theme();
    h_flex()
        .pt_2()
        .border_t_1()
        .border_color(theme.border.opacity(0.5))
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(div().flex_1().child(format!(
            "Status bar every {}s · details every {}s",
            settings.interval_secs, settings.detail_interval_secs
        )))
        .child(
            Button::new("host-monitor-settings")
                .ghost()
                .xsmall()
                .icon(IconName::Settings2)
                .tooltip("Monitor settings")
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(OpenMonitorSettings), cx)
                }),
        )
        .into_any_element()
}

/// A titled block with its headline on the right.
fn block(
    title: &'static str,
    headline: impl IntoElement,
    content: impl IntoElement,
    cx: &App,
) -> AnyElement {
    v_flex()
        .gap_1()
        .child(
            h_flex()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .font_semibold()
                        .text_color(cx.theme().muted_foreground)
                        .child(title.to_uppercase()),
                )
                .child(headline),
        )
        .child(content)
        .into_any_element()
}

fn strong(text: String) -> impl IntoElement {
    div().text_sm().font_semibold().child(text)
}

#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn section(
    metric: MonitorMetric,
    snapshot: &Snapshot,
    history: &History,
    span: f64,
    cx: &App,
) -> Option<AnyElement> {
    let theme = cx.theme();
    Some(match metric {
        MonitorMetric::Cpu => {
            let cpu = snapshot.cpu?;
            block(
                "CPU",
                strong(format::percent(cpu)),
                graph::sparkline(
                    vec![Line::new(&history.cpu, theme.chart_1)],
                    span,
                    Some(100.0),
                    cx,
                ),
                cx,
            )
        }
        MonitorMetric::Cores => {
            let cores = snapshot.cores.as_ref()?;
            block(
                "Cores",
                strong(format!("{}", cores.len())),
                cores_grid(cores, theme.chart_1, cx),
                cx,
            )
        }
        MonitorMetric::Memory => {
            let memory = snapshot.memory?;
            block(
                "Memory",
                strong(format!(
                    "{} / {}",
                    format::bytes(memory.used),
                    format::bytes(memory.total)
                )),
                v_flex()
                    .gap_1()
                    .child(graph::meter(memory.percent(), theme.chart_2, cx))
                    .child(graph::sparkline(
                        vec![Line::new(&history.memory, theme.chart_2)],
                        span,
                        Some(100.0),
                        cx,
                    )),
                cx,
            )
        }
        MonitorMetric::Swap => {
            let swap = snapshot.swap.filter(|swap| swap.total > 0)?;
            block(
                "Swap",
                strong(format!(
                    "{} / {}",
                    format::bytes(swap.used),
                    format::bytes(swap.total)
                )),
                graph::meter(swap.percent(), theme.chart_4, cx),
                cx,
            )
        }
        MonitorMetric::Temperature => {
            let sensors = snapshot
                .temperatures
                .as_ref()
                .filter(|list| !list.is_empty())?;
            let hottest = snapshot.hottest()?.celsius;
            block(
                "Temperature",
                strong(format::celsius(hottest)),
                div()
                    .grid()
                    .grid_cols(2)
                    .gap_x_4()
                    .gap_y_0p5()
                    .children(sensors.iter().map(|sensor| {
                        h_flex()
                            .gap_2()
                            .text_xs()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(theme.muted_foreground)
                                    .child(sensor.label.clone()),
                            )
                            .child(
                                div()
                                    .text_color(temperature_color(sensor.celsius, cx))
                                    .child(format::celsius(sensor.celsius)),
                            )
                    })),
                cx,
            )
        }
        MonitorMetric::Disks => {
            let disks = snapshot
                .filesystems
                .as_ref()
                .filter(|list| !list.is_empty())?;
            block(
                "Disks",
                div(),
                v_flex().gap_1p5().children(disks.iter().map(|disk| {
                    v_flex()
                        .gap_0p5()
                        .child(
                            h_flex()
                                .gap_2()
                                .text_xs()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .child(disk.mount.clone()),
                                )
                                .child(div().text_color(theme.muted_foreground).child(format!(
                                    "{} / {} · {}",
                                    format::bytes(disk.used),
                                    format::bytes(disk.total),
                                    format::percent(disk.percent())
                                ))),
                        )
                        .child(graph::meter(disk.percent(), theme.chart_3, cx))
                })),
                cx,
            )
        }
        MonitorMetric::Network => {
            let network = snapshot.network?;
            block(
                "Network",
                h_flex()
                    .gap_3()
                    .child(graph::figure(
                        "↓",
                        format::rate(network.received),
                        Some(theme.chart_3),
                        cx,
                    ))
                    .child(graph::figure(
                        "↑",
                        format::rate(network.sent),
                        Some(theme.chart_4),
                        cx,
                    )),
                graph::sparkline(
                    vec![
                        Line::new(&history.received, theme.chart_3),
                        Line::new(&history.sent, theme.chart_4),
                    ],
                    span,
                    None,
                    cx,
                ),
                cx,
            )
        }
        MonitorMetric::DiskIo => {
            let disk = snapshot.disk_io?;
            block(
                "Disk I/O",
                h_flex()
                    .gap_3()
                    .child(graph::figure(
                        "read",
                        format::rate(disk.read),
                        Some(theme.chart_5),
                        cx,
                    ))
                    .child(graph::figure(
                        "write",
                        format::rate(disk.written),
                        Some(theme.chart_2),
                        cx,
                    )),
                graph::sparkline(
                    vec![
                        Line::new(&history.read, theme.chart_5),
                        Line::new(&history.written, theme.chart_2),
                    ],
                    span,
                    None,
                    cx,
                ),
                cx,
            )
        }
        // Shown in the header.
        MonitorMetric::Load | MonitorMetric::Uptime => return None,
    })
}

/// A cell per core: its number, its load and a bar.
fn cores_grid(cores: &[f32], color: Hsla, cx: &App) -> impl IntoElement {
    let columns = if cores.len() > 16 { 8 } else { 4 };
    let theme = cx.theme();
    div()
        .grid()
        .grid_cols(columns)
        .gap_x_3()
        .gap_y_1()
        .children(cores.iter().enumerate().map(|(index, load)| {
            v_flex()
                .gap_0p5()
                .child(
                    h_flex()
                        .text_xs()
                        .child(
                            div()
                                .flex_1()
                                .text_color(theme.muted_foreground)
                                .child(format!("{index}")),
                        )
                        .child(format::percent(*load)),
                )
                .child(graph::meter(*load, color, cx))
        }))
}

fn temperature_color(celsius: f32, cx: &App) -> Hsla {
    let theme = cx.theme();
    if celsius >= 85.0 {
        theme.danger
    } else if celsius >= 70.0 {
        theme.warning
    } else {
        theme.foreground
    }
}
