//! The Monitor settings page: what to show where, and how often.

use std::{ops::RangeInclusive, time::Duration};

use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, SharedString, Subscription,
    Task, Window,
    component::{
        Sizable as _,
        input::{InputEvent, InputState, NumberInput, NumberInputEvent, StepAction},
        switch::Switch,
        v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_settings::{
    MONITOR_DETAIL_INTERVAL_RANGE, MONITOR_HISTORY_RANGE, MONITOR_INTERVAL_RANGE, MonitorMetric,
    MonitorSettings, Settings,
};
use nocterm_ui::{ActiveSettings as _, SettingsStore, edit_settings, form};
use nocterm_workspace::SettingsPage;

/// How long typing must pause before a number saves.
const DEBOUNCE: Duration = Duration::from_millis(600);

type Field = fn(&mut MonitorSettings) -> &mut u32;

/// A whole number setting that saves itself.
struct Number {
    input: Entity<InputState>,
    field: Field,
    range: RangeInclusive<u32>,
    error: Option<SharedString>,
    debounce: Option<Task<()>>,
    _subscriptions: [Subscription; 2],
}

impl Number {
    /// The `index`th number on the page, editing `field`.
    fn new(
        index: usize,
        field: Field,
        range: RangeInclusive<u32>,
        window: &mut Window,
        cx: &mut Context<MonitorPage>,
    ) -> Self {
        let value = *field(&mut cx.settings().monitor.clone());
        let input = cx.new(|cx| InputState::new(window, cx).default_value(value.to_string()));
        let typed = cx.subscribe_in(&input, window, move |page, _, event, window, cx| {
            let number = &mut page.numbers[index];
            match event {
                InputEvent::Change => {
                    number.debounce = Some(cx.spawn_in(window, async move |page, cx| {
                        cx.background_executor().timer(DEBOUNCE).await;
                        let _ = page.update(cx, |page, cx| page.commit(index, cx));
                    }));
                }
                InputEvent::Blur | InputEvent::PressEnter { .. } => page.commit(index, cx),
                InputEvent::Focus => {}
            }
        });
        let stepped = cx.subscribe_in(&input, window, move |page, input, event, window, cx| {
            let NumberInputEvent::Step(step) = event;
            let number = &page.numbers[index];
            let current = input
                .read(cx)
                .value()
                .trim()
                .parse::<u32>()
                .unwrap_or(*number.range.start());
            let next = match step {
                StepAction::Increment => current.saturating_add(1),
                StepAction::Decrement => current.saturating_sub(1),
            }
            .clamp(*number.range.start(), *number.range.end());
            input.update(cx, |input, cx| {
                input.set_value(next.to_string(), window, cx)
            });
            page.commit(index, cx);
        });
        Self {
            input,
            field,
            range,
            error: None,
            debounce: None,
            _subscriptions: [typed, stepped],
        }
    }

    /// Shows a value saved elsewhere, unless the user is editing this field.
    fn sync(&mut self, window: &mut Window, cx: &mut App) {
        let value = (self.field)(&mut cx.settings().monitor.clone()).to_string();
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        if focused || self.debounce.is_some() || self.input.read(cx).value() == value.as_str() {
            return;
        }
        self.error = None;
        self.input
            .update(cx, |input, cx| input.set_value(value, window, cx));
    }
}

pub struct MonitorPage {
    focus: FocusHandle,
    numbers: Vec<Number>,
    _settings: Subscription,
}

impl MonitorPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let numbers = vec![
            Number::new(
                0,
                |monitor| &mut monitor.interval_secs,
                MONITOR_INTERVAL_RANGE,
                window,
                cx,
            ),
            Number::new(
                1,
                |monitor| &mut monitor.detail_interval_secs,
                MONITOR_DETAIL_INTERVAL_RANGE,
                window,
                cx,
            ),
            Number::new(
                2,
                |monitor| &mut monitor.history_points,
                MONITOR_HISTORY_RANGE,
                window,
                cx,
            ),
        ];
        let settings = cx.observe_global_in::<SettingsStore>(window, |page, window, cx| {
            for number in &mut page.numbers {
                number.sync(window, cx);
            }
            cx.notify();
        });
        Self {
            focus: cx.focus_handle(),
            numbers,
            _settings: settings,
        }
    }

    /// Saves the `index`th number's text if it is a number in range.
    fn commit(&mut self, index: usize, cx: &mut Context<Self>) {
        let number = &mut self.numbers[index];
        let field = number.field;
        number.debounce = None;
        let text = number.input.read(cx).value().trim().to_owned();
        let (start, end) = (*number.range.start(), *number.range.end());
        match text.parse::<u32>() {
            Ok(value) if number.range.contains(&value) => {
                number.error = None;
                if *field(&mut cx.settings().monitor.clone()) != value {
                    edit_settings(cx, move |settings| *field(&mut settings.monitor) = value)
                        .detach();
                }
            }
            _ => number.error = Some(format!("Enter a whole number from {start} to {end}.").into()),
        }
        cx.notify();
    }

    fn number_row(
        &self,
        index: usize,
        label: &'static str,
        description: &'static str,
        unit: &'static str,
        cx: &App,
    ) -> gpui_kit::AnyElement {
        let number = &self.numbers[index];
        v_flex()
            .child(form::row(
                label,
                description,
                div().w(rems(8.)).child(
                    NumberInput::new(&number.input)
                        .small()
                        .suffix(div().text_xs().child(unit)),
                ),
                cx,
            ))
            .when_some(number.error.clone(), |row, error| {
                row.child(form::error_text(error, cx))
            })
            .into_any_element()
    }
}

/// Turns `metric` on or off in `list`. A list in the usual order stays in
/// it; a list the user ordered by hand gets the new metric at its end.
pub(crate) fn toggle(list: &mut Vec<MonitorMetric>, metric: MonitorMetric, on: bool) {
    list.retain(|item| *item != metric);
    if !on {
        return;
    }
    let rank = |metric: &MonitorMetric| MonitorMetric::ALL.iter().position(|m| m == metric);
    if list.is_sorted_by_key(rank) {
        let at = list.partition_point(|item| rank(item) < rank(&metric));
        list.insert(at, metric);
    } else {
        list.push(metric);
    }
}

fn switch(
    id: impl Into<gpui_kit::ElementId>,
    checked: bool,
    edit: impl Fn(&mut Settings, bool) + Clone + 'static,
    cx: &mut Context<MonitorPage>,
) -> Switch {
    Switch::new(id)
        .checked(checked)
        .on_click(cx.listener(move |_, checked: &bool, _, cx| {
            let (checked, edit) = (*checked, edit.clone());
            edit_settings(cx, move |settings| edit(settings, checked)).detach();
            cx.notify();
        }))
}

fn description(metric: MonitorMetric) -> &'static str {
    match metric {
        MonitorMetric::Cpu => "Total processor load.",
        MonitorMetric::Cores => "Load of every processor core.",
        MonitorMetric::Memory => "Memory in use out of the total.",
        MonitorMetric::Swap => "Swap or page file in use.",
        MonitorMetric::Load => "Run queue averages over 1, 5 and 15 minutes (Linux).",
        MonitorMetric::Uptime => "Time since the host started.",
        MonitorMetric::Temperature => "Every temperature sensor the host exposes.",
        MonitorMetric::Disks => "Space used on each file system.",
        MonitorMetric::Network => "Download and upload speed, with a graph.",
        MonitorMetric::DiskIo => "Disk read and write speed, with a graph.",
    }
}

impl Render for MonitorPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let monitor = cx.settings().monitor.clone();
        let general = [
            form::row(
                "Show the host monitor",
                "Resources of the active host at the start of the status bar.",
                switch(
                    "monitor-enabled",
                    monitor.enabled,
                    |s, on| s.monitor.enabled = on,
                    cx,
                ),
                cx,
            ),
            form::row(
                "Watch this computer",
                "Monitor the local machine while no remote session is active.",
                switch(
                    "monitor-local",
                    monitor.local,
                    |s, on| s.monitor.local = on,
                    cx,
                ),
                cx,
            ),
        ];
        let refresh = [
            self.number_row(
                0,
                "Status bar refresh",
                "While the details are closed, only what the status bar shows is collected.",
                "s",
                cx,
            ),
            self.number_row(
                1,
                "Details refresh",
                "While the details are open, everything they show is collected.",
                "s",
                cx,
            ),
            self.number_row(
                2,
                "Graph length",
                "Samples each graph in the details keeps.",
                "pts",
                cx,
            ),
        ];
        let rows = |prefix: &'static str,
                    list: &[MonitorMetric],
                    pick: fn(&mut MonitorSettings) -> &mut Vec<MonitorMetric>,
                    cx: &mut Context<Self>| {
            MonitorMetric::ALL
                .into_iter()
                .map(|metric| {
                    form::row(
                        metric.label(),
                        description(metric),
                        switch(
                            SharedString::from(format!("{prefix}-{metric:?}")),
                            list.contains(&metric),
                            move |s, on| toggle(pick(&mut s.monitor), metric, on),
                            cx,
                        ),
                        cx,
                    )
                })
                .collect::<Vec<_>>()
        };
        let status_bar = rows(
            "monitor-status",
            &monitor.status_bar,
            |monitor| &mut monitor.status_bar,
            cx,
        );
        let details = rows(
            "monitor-details",
            &monitor.details,
            |monitor| &mut monitor.details,
            cx,
        );
        v_flex()
            .w_full()
            .track_focus(&self.focus)
            .child(form::page_header(
                "Monitor",
                Some("CPU, memory, disks, network and temperatures of the host in the active tab, on Linux and Windows.".into()),
                cx,
            ))
            .child(form::section("General", general, cx))
            .child(form::section("Refresh", refresh, cx))
            .child(form::section("Status bar", status_bar, cx))
            .child(form::section("Details", details, cx))
    }
}

impl Focusable for MonitorPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SettingsPage for MonitorPage {}

#[cfg(test)]
mod tests {
    use super::*;
    use MonitorMetric::*;

    #[test]
    fn toggling_keeps_the_usual_order_or_the_users() {
        let mut list = vec![Cpu, Uptime];
        toggle(&mut list, Memory, true);
        assert_eq!(list, [Cpu, Memory, Uptime]);
        toggle(&mut list, Cpu, false);
        assert_eq!(list, [Memory, Uptime]);
        let mut custom = vec![Memory, Cpu];
        toggle(&mut custom, Network, true);
        assert_eq!(custom, [Memory, Cpu, Network]);
        toggle(&mut custom, Network, true);
        assert_eq!(custom, [Memory, Cpu, Network], "no duplicates");
    }
}
