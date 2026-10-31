//! A compact table of what the person's own agents consumed per model over the last month.

use gpui_kit::{
    App, Task, TestSupportExt as _,
    component::{ActiveTheme as _, h_flex, v_flex},
    div,
    prelude::*,
};
use nocterm_cloud_agents_model::ModelUsage;

use crate::layout::{field, note};
use crate::servers::item;

/// The window the table covers.
pub(super) const USAGE_DAYS: u32 = 30;

/// The rows of the table and the read that fills them.
#[derive(Default)]
pub(super) struct UsageTable {
    rows: Vec<ModelUsage>,
    reading: Option<Task<()>>,
}

impl UsageTable {
    /// Forgets the rows and any read under way (another server is shown).
    pub(super) fn clear(&mut self) {
        self.rows.clear();
        self.reading = None;
    }

    /// Replaces the rows with what the hub reported.
    pub(super) fn show(&mut self, rows: Vec<ModelUsage>) {
        self.rows = rows;
    }

    /// Keeps the read alive until it ends or the table is cleared.
    pub(super) fn reading(&mut self, task: Task<()>) {
        self.reading = Some(task);
    }

    pub(super) fn render(&self, key: &str, cx: &App) -> Option<impl IntoElement> {
        if self.rows.is_empty() {
            return None;
        }
        let mono = cx.theme().mono_font_family.clone();
        let rows = self.rows.iter().map(|usage| {
            h_flex()
                .id(item(&format!("cloud-agents-usage-{}", usage.label()), key))
                .test_support()
                .gap_3()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(usage.label().to_owned()),
                )
                .child(div().font_family(mono.clone()).text_sm().child(format!(
                    "{} runs  {} in / {} out  {}",
                    usage.runs,
                    usage.input_tokens,
                    usage.output_tokens,
                    spend(usage.spend_micros)
                )))
        });
        Some(
            v_flex()
                .id(item("cloud-agents-usage", key))
                .test_support()
                .gap_2()
                .child(note(
                    format!("Your own agents over the last {USAGE_DAYS} days, per model."),
                    cx,
                ))
                .child(field("Usage", v_flex().gap_1().children(rows))),
        )
    }
}

/// Millionths of the currency unit as a decimal with up to four places.
fn spend(micros: u64) -> String {
    let (whole, part) = (micros / 1_000_000, micros % 1_000_000 / 100);
    format!("{whole}.{part:04}")
}

#[cfg(test)]
mod tests {
    use super::spend;

    #[test]
    fn spend_shows_four_decimal_places() {
        assert_eq!(spend(0), "0.0000");
        assert_eq!(spend(1_234_567), "1.2345");
        assert_eq!(spend(900), "0.0009");
    }
}
