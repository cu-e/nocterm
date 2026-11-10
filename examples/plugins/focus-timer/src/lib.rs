//! The focus timer: an example code plugin for nocterm.
//!
//! The plugin value holds no state. The start of the round and the count of finished rounds
//! live in the plugin's stored data (`storage.set`), which the host hands back in every
//! snapshot, so an instance the host restarts after a fault carries on where it was.

use nocterm_plugin_sdk::{
    Cx, Plugin, ViewBuilder,
    api::{Effects, Event, LocalId, Refresh, Tone, View},
    effect,
    observe::Clock,
};

const BAR: &str = "bar";
const PANEL: &str = "panel";
const START: &str = "start";
const ROUNDS: &str = "rounds";
const DEFAULT_GOAL: i64 = 25;

#[derive(Default)]
pub struct FocusTimer;

/// Minutes since the stored start, when the clock is readable and a round has started.
fn elapsed(cx: &Cx<'_>) -> Option<i64> {
    let now = cx.observe::<Clock>().ok()?.unix_minutes;
    let start = cx.stored(START)?.parse::<i64>().ok()?;
    Some((now - start).max(0))
}

fn goal(cx: &Cx<'_>) -> i64 {
    cx.setting("goal")
        .and_then(|value| value.as_number())
        .map_or(DEFAULT_GOAL, i64::from)
        .max(1)
}

fn rounds(cx: &Cx<'_>) -> u16 {
    cx.stored(ROUNDS)
        .and_then(|text| text.parse().ok())
        .unwrap_or(0)
}

impl FocusTimer {
    fn bar(cx: &Cx<'_>) -> View {
        let mut view = ViewBuilder::new();
        let row = view.row(None);
        let label = match elapsed(cx) {
            Some(minutes) => format!("Focus {minutes}m"),
            None => String::from("Focus"),
        };
        view.button(row, "open", label);
        view.refresh(Refresh::NextMinute);
        view.build()
    }

    fn panel(cx: &Cx<'_>) -> View {
        let mut view = ViewBuilder::new();
        let column = view.column(None);
        let goal = goal(cx);
        match elapsed(cx) {
            Some(minutes) => {
                view.text(column, format!("{minutes} of {goal} min"));
                let percent = (minutes * 100 / goal).clamp(0, 100) as u8;
                let meter = view.meter(column, percent);
                if minutes >= goal {
                    view.tone(meter, Tone::Success);
                }
            }
            None => {
                let note = view.text(column, "No round yet. Press Reset to start one.");
                view.tone(note, Tone::Muted);
            }
        }
        let row = view.row(column);
        view.button(row, "reset", "Reset");
        view.button(row, "close", "Close");
        view.refresh(Refresh::NextMinute);
        view.build()
    }

    fn open(cx: &Cx<'_>) -> Effects {
        let mut requests = Vec::new();
        if cx.stored(START).is_none()
            && let Ok(now) = cx.observe::<Clock>()
        {
            requests.push(effect::store(START, now.unix_minutes.to_string()));
        }
        if let Ok(panel) = LocalId::parse(PANEL) {
            requests.push(effect::open_panel(panel, true));
        }
        if let Ok(bar) = LocalId::parse(BAR) {
            // The finished rounds stay on the badge; a render cannot set it, so it comes back
            // whenever the item is pressed.
            requests.push(effect::badge(
                bar,
                Some(rounds(cx)).filter(|count| *count > 0),
            ));
        }
        effect::effects(requests)
    }

    fn reset(cx: &Cx<'_>) -> Effects {
        let Ok(now) = cx.observe::<Clock>() else {
            return Effects::default();
        };
        let mut requests = vec![effect::store(START, now.unix_minutes.to_string())];
        if elapsed(cx).is_some_and(|minutes| minutes >= goal(cx)) {
            let finished = rounds(cx).saturating_add(1);
            requests.push(effect::store(ROUNDS, finished.to_string()));
            if let Ok(bar) = LocalId::parse(BAR) {
                requests.push(effect::badge(bar, Some(finished)));
            }
        }
        effect::effects(requests)
    }

    fn close() -> Effects {
        match LocalId::parse(PANEL) {
            Ok(panel) => effect::effects([effect::open_panel(panel, false)]),
            Err(_) => Effects::default(),
        }
    }
}

impl Plugin for FocusTimer {
    fn render(&mut self, cx: &mut Cx<'_>) -> View {
        if cx.local().as_str() == PANEL {
            Self::panel(cx)
        } else {
            Self::bar(cx)
        }
    }

    fn on_event(&mut self, cx: &mut Cx<'_>, event: &Event) -> Effects {
        let Event::Activated { key } = event else {
            return Effects::default();
        };
        match (cx.local().as_str(), key.as_str()) {
            (BAR, "open") => Self::open(cx),
            (PANEL, "reset") => Self::reset(cx),
            (PANEL, "close") => Self::close(),
            _ => Effects::default(),
        }
    }
}

nocterm_plugin_sdk::export!(
    FocusTimer,
    concat!(env!("CARGO_MANIFEST_DIR"), "/plugin.toml")
);
