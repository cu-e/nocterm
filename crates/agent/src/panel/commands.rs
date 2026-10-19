//! ACP command completion and details, with local keyboard navigation.
use super::{
    AgentPanel, MenuKind,
    widgets::{chat_markdown, safe_markdown},
};
use gpui_kit::{
    AnyElement, AnyView, App, Context, KeyBinding, ListAlignment, ListOffset, ListSizingBehavior,
    ListState, TestSupportExt as _, WeakEntity,
    component::{
        ActiveTheme as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        input::{IndentInline, MoveDown, MoveUp, OutdentInline},
        tooltip::Tooltip,
        v_flex,
    },
    div, list,
    prelude::*,
    px,
};
use nocterm_ai::model::{
    controls::{ConfigCategory, ConfigKind, ConfigOption, SelectChoices},
    notice::AdvertisedCommand,
};

pub(super) struct CommandState {
    selected: usize,
    scroll: ListState,
    visible_matches: Vec<usize>,
    dismissed: Option<String>,
    cycle: Option<CommandCycle>,
    token_prefix: Option<String>,
    input_value: String,
    advertisement: Option<(gpui_kit::EntityId, u64)>,
}
impl Default for CommandState {
    fn default() -> Self {
        Self {
            selected: 0,
            scroll: ListState::new(0, ListAlignment::Top, px(200.)),
            visible_matches: Vec::new(),
            dismissed: None,
            cycle: None,
            token_prefix: None,
            input_value: String::new(),
            advertisement: None,
        }
    }
}
impl CommandState {
    pub(super) fn reset(&mut self) {
        self.selected = 0;
        self.reveal(0);
        self.cycle = None;
        self.dismissed = None;
        self.token_prefix = None;
        self.input_value.clear();
        self.advertisement = None;
        self.visible_matches.clear();
        self.scroll.reset(0);
    }
    pub(super) fn observe_input(&mut self, value: String) -> bool {
        if self.input_value == value {
            return false;
        }
        self.input_value = value.clone();
        if !self
            .cycle
            .as_ref()
            .is_some_and(|cycle| cycle.inserted == value)
        {
            self.cycle = None;
            self.selected = 0;
            self.reveal(0);
            self.dismissed = None;
        }
        true
    }
    pub(super) fn observe_advertisement(&mut self, key: (gpui_kit::EntityId, u64)) -> bool {
        if self.advertisement == Some(key) {
            return false;
        }
        self.advertisement = Some(key);
        self.visible_matches.clear();
        self.scroll.reset(0);
        self.cycle = None;
        self.selected = 0;
        self.reveal(0);
        self.token_prefix = None;
        true
    }
    pub(super) fn matches(&self, value: &str, commands: &[AdvertisedCommand]) -> Vec<usize> {
        if self.dismissed.as_deref() == Some(value) {
            return Vec::new();
        }
        let filter = self
            .cycle
            .as_ref()
            .filter(|cycle| cycle.inserted == value)
            .map(|cycle| cycle.filter.as_str())
            .unwrap_or(value);
        let Some(prefix) = nocterm_ai::commands::prefix(filter) else {
            return Vec::new();
        };
        let prefix = prefix.to_lowercase();
        commands
            .iter()
            .enumerate()
            .filter_map(|(index, command)| {
                command
                    .name
                    .to_lowercase()
                    .starts_with(&prefix)
                    .then_some(index)
            })
            .collect()
    }
    pub(super) fn observe_token(&mut self, prefix: Option<String>) -> bool {
        if self.token_prefix == prefix {
            return false;
        }
        self.token_prefix = prefix;
        true
    }
    pub(super) fn action(&mut self, key: &str, value: &str, names: &[String]) -> Option<String> {
        if names.is_empty() {
            return None;
        }
        let index = self.selected.min(names.len() - 1);
        let chosen = match key {
            "up" => {
                self.selected = (index + names.len() - 1) % names.len();
                None
            }
            "down" => {
                self.selected = (index + 1) % names.len();
                None
            }
            "escape" => {
                self.dismissed = Some(value.to_owned());
                self.cycle = None;
                None
            }
            "tab" | "shift-tab" => {
                let filter = self
                    .cycle
                    .as_ref()
                    .map(|cycle| cycle.filter.clone())
                    .unwrap_or_else(|| value.to_owned());
                let next = if key == "shift-tab" {
                    (index + names.len() - 1) % names.len()
                } else if self.cycle.is_some() {
                    (index + 1) % names.len()
                } else {
                    index
                };
                self.selected = next;
                self.cycle = Some(CommandCycle {
                    filter,
                    inserted: format!("/{} ", names[next]),
                });
                Some(names[next].clone())
            }
            "enter" => {
                self.cycle = None;
                Some(names[index].clone())
            }
            _ => None,
        };
        self.reveal(self.selected);
        chosen
    }
    pub(super) fn confirm(&mut self) {
        self.cycle = None;
    }
    pub(super) fn selected(&self) -> usize {
        self.selected
    }
    pub(super) fn list_state(&mut self, matches: &[usize]) -> ListState {
        if self.visible_matches != matches {
            self.scroll.reset(matches.len());
            self.visible_matches = matches.to_vec();
            self.reveal(self.selected.min(matches.len().saturating_sub(1)));
        }
        self.scroll.clone()
    }
    pub(super) fn reveal(&self, index: usize) {
        if self.scroll.bounds_for_item(index).is_some() {
            self.scroll.scroll_to_reveal_item(index);
        } else {
            // Unmeasured destinations (e.g. Shift+Tab wrapping) have no height yet.
            self.scroll.scroll_to(ListOffset {
                item_ix: index,
                offset_in_item: px(0.),
            });
        }
    }
}
pub(super) struct CommandCycle {
    filter: String,
    inserted: String,
}
pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", MoveUp, Some("AgentSlashCommands")),
        KeyBinding::new("down", MoveDown, Some("AgentSlashCommands")),
        KeyBinding::new("tab", IndentInline, Some("AgentSlashCommands")),
        KeyBinding::new("shift-tab", OutdentInline, Some("AgentSlashCommands")),
    ]);
}
#[cfg(test)]
pub(super) fn matches<'a>(
    text: &str,
    commands: &'a [AdvertisedCommand],
) -> Vec<&'a AdvertisedCommand> {
    let Some(prefix) = nocterm_ai::commands::prefix(text) else {
        return Vec::new();
    };
    let prefix = prefix.to_lowercase();
    commands
        .iter()
        .filter(|command| command.name.to_lowercase().starts_with(&prefix))
        .collect()
}
fn summary(text: &str) -> String {
    let mut result = String::new();
    let mut pending_space = false;
    let mut count = 0;
    for ch in text.chars().take(4096) {
        if ch.is_whitespace() {
            pending_space = !result.is_empty();
            continue;
        }
        if pending_space {
            if count == 160 {
                result.push('…');
                break;
            }
            result.push(' ');
            count += 1;
            pending_space = false;
        }
        if count == 160 {
            result.push('…');
            break;
        }
        result.push(ch);
        count += 1;
    }
    result
}

fn input_hint(command: &AdvertisedCommand) -> Option<&str> {
    command.input_hint.as_deref()
}
pub(super) fn config_hint(option: &ConfigOption) -> String {
    if option.category != Some(ConfigCategory::Model) {
        return option.name.clone();
    }
    let count = match &option.kind {
        ConfigKind::Select { choices, .. } => match choices {
            SelectChoices::Flat(values) => values.len(),
            SelectChoices::Groups(groups) => groups.iter().map(|group| group.choices.len()).sum(),
        },
        _ => 0,
    };
    format!("Choose model · {count} available · open to browse or search")
}
pub(super) fn details(command: &AdvertisedCommand, cx: &App) -> AnyElement {
    v_flex()
        .id("command-details")
        .test_support()
        .w_full()
        .min_w_0()
        .gap_1()
        .p_2()
        .child(div().text_xs().child(format!("/{}", command.name)))
        .child(
            v_flex()
                .id("command-description")
                .test_support()
                .max_h(px(180.))
                .overflow_y_scroll()
                .gap_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(
                    chat_markdown(
                        "command-description-text",
                        safe_markdown(&command.description),
                    )
                    .text_xs(),
                )
                .when_some(input_hint(command), |body, hint| {
                    body.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().foreground)
                            .child("Arguments"),
                    )
                    .child(chat_markdown("command-input-hint", safe_markdown(hint)).text_xs())
                }),
        )
        .into_any_element()
}
/// Shared hover card for list rows and the actual inline command token.
pub(super) fn tooltip(name: String, panel: WeakEntity<AgentPanel>, cx: &mut App) -> AnyView {
    let command = panel
        .upgrade()
        .and_then(|panel| panel.read(cx).current())
        .and_then(|thread| {
            thread
                .read(cx)
                .configuration()
                .commands()
                .iter()
                .find(|command| command.name == name)
                .cloned()
        });
    let model_option = (name == "model")
        .then(|| panel.upgrade())
        .flatten()
        .and_then(|panel| panel.read(cx).current())
        .and_then(|thread| {
            thread
                .read(cx)
                .config_options()
                .iter()
                .find(|option| option.category == Some(ConfigCategory::Model))
                .map(|option| option.id.to_string())
        });
    cx.new(|_| {
        Tooltip::element(move |_, cx| {
            let panel = panel.clone();
            v_flex()
                .w(px(320.))
                .max_w_full()
                .when_some(command.clone(), |body, command| {
                    body.child(details(&command, cx))
                })
                .when_some(model_option.clone(), |body, id| {
                    body.child(
                        Button::new("command-browse-models")
                            .ghost()
                            .small()
                            .label("Browse available models…")
                            .on_click(move |_, _, cx| {
                                let _ = panel.update(cx, |panel, cx| {
                                    panel.toggle_menu(MenuKind::Config(id.clone()), cx)
                                });
                            }),
                    )
                })
        })
    })
    .into()
}
#[derive(Clone, Copy)]
pub(super) struct CommandOrigin {
    chat: gpui_kit::EntityId,
    advertisement: u64,
}
pub(super) struct CommandRenderer;
impl CommandRenderer {
    fn row(
        index: usize,
        command: &AdvertisedCommand,
        selected: bool,
        origin: CommandOrigin,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let name = command.name.clone();
        let hover = name.clone();
        let owner = cx.weak_entity();
        div()
            .id(("command-hover", index))
            .test_support()
            .aria_selected(selected)
            .pb_1()
            .w_full()
            .hoverable_tooltip(move |_, cx| tooltip(hover.clone(), owner.clone(), cx))
            .child(
                Button::new(("slash-command", index))
                    .ghost()
                    .small()
                    .w_full()
                    .h_auto()
                    .accessibility_label(format!("/{}", command.name))
                    .when(selected, |button| button.bg(cx.theme().accent))
                    .child(
                        v_flex()
                            .id(("command-summary", index))
                            .test_support()
                            .w_full()
                            .min_w_0()
                            .text_left()
                            .py_2()
                            .px_2()
                            .gap_1()
                            .child(div().truncate().child(format!("/{}", command.name)))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .truncate()
                                    .child(summary(&command.description)),
                            )
                            .when_some(input_hint(command), |column, hint| {
                                column.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .truncate()
                                        .child(summary(hint)),
                                )
                            }),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.on_command_selected(origin, name.clone(), window, cx);
                    })),
            )
            .into_any_element()
    }
}

impl CommandRenderer {
    pub(super) fn list(
        scroll: gpui_kit::ListState,
        rows: Vec<AdvertisedCommand>,
        selected: usize,
        origin: CommandOrigin,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let menu = div()
            .id("agent-slash-commands")
            .test_support()
            .rounded(px(8.))
            .bg(cx.theme().muted)
            .p_1()
            .min_w_0()
            .child(
                list(
                    scroll,
                    cx.processor(move |_, index, _, cx| {
                        Self::row(index, &rows[index], index == selected, origin, cx)
                    }),
                )
                .with_sizing_behavior(ListSizingBehavior::Infer)
                .w_full()
                .max_h(px(192.)),
            );
        div().mx_3().min_w_0().child(menu).into_any_element()
    }
}

impl CommandOrigin {
    pub(super) fn new(chat: gpui_kit::EntityId, advertisement: u64) -> Self {
        Self {
            chat,
            advertisement,
        }
    }
}

impl CommandOrigin {
    pub(super) fn matches(&self, chat: gpui_kit::EntityId, advertisement: u64) -> bool {
        self.chat == chat && self.advertisement == advertisement
    }
}

#[cfg(test)]
mod tests {
    use super::summary;
    #[test]
    fn summary_is_one_line_and_bounded_without_cutting_unicode() {
        assert_eq!(summary("first\n\tsecond   third"), "first second third");
        let long = summary(&"я".repeat(300));
        assert_eq!(long.chars().count(), 161);
        assert!(long.ends_with('…'));
    }
}
