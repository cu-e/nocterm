//! ACP command completion and details, with local keyboard navigation.
use super::{
    AgentPanel, MenuKind,
    widgets::{chat_markdown, safe_markdown},
};
use gpui_kit::{
    AnyElement, AnyView, App, Context, KeyBinding, ListAlignment, ListOffset, ListSizingBehavior,
    ListState, TestSupportExt as _, WeakEntity, Window,
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
use nocterm_ai::acp;

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
    pub(super) fn matches(&self, value: &str, commands: &[acp::AvailableCommand]) -> Vec<usize> {
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
    commands: &'a [acp::AvailableCommand],
) -> Vec<&'a acp::AvailableCommand> {
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

fn input_hint(command: &acp::AvailableCommand) -> Option<&str> {
    match command.input.as_ref()? {
        acp::AvailableCommandInput::Unstructured(input) => Some(&input.hint),
        _ => None,
    }
}
pub(super) fn config_hint(option: &acp::SessionConfigOption) -> String {
    if option.category != Some(acp::SessionConfigOptionCategory::Model) {
        return option.name.clone();
    }
    let count = match &option.kind {
        acp::SessionConfigKind::Select(select) => match &select.options {
            acp::SessionConfigSelectOptions::Ungrouped(values) => values.len(),
            acp::SessionConfigSelectOptions::Grouped(groups) => {
                groups.iter().map(|group| group.options.len()).sum()
            }
            _ => 0,
        },
        _ => 0,
    };
    format!("Choose model · {count} available · open to browse or search")
}
pub(super) fn details(command: &acp::AvailableCommand, cx: &App) -> AnyElement {
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
                .find(|option| option.category == Some(acp::SessionConfigOptionCategory::Model))
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
impl AgentPanel {
    pub(super) fn reset_commands(&mut self) {
        self.composer.reset_commands();
    }
    pub(super) fn refresh_commands(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = self.current() else {
            return;
        };
        let key = (
            thread.entity_id(),
            thread.read(cx).configuration().commands_revision(),
        );
        let commands = thread.read(cx).configuration().commands().to_vec();
        if self.composer.refresh_commands(key, &commands, window, cx) {
            cx.notify();
        }
    }
    pub(super) fn command_matches(&self, cx: &App) -> Vec<usize> {
        self.current()
            .map(|thread| {
                self.composer
                    .command_matches(thread.read(cx).configuration().commands(), cx)
            })
            .unwrap_or_default()
    }
    pub(super) fn sync_command_token(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let commands = self
            .current()
            .map(|thread| thread.read(cx).configuration().commands().to_vec())
            .unwrap_or_default();
        self.composer.sync_command_token(&commands, window, cx);
    }
    fn complete_command(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.confirm_command(name, window, cx);
        cx.notify();
    }
    pub(super) fn command_action(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.composer.focus(cx).is_focused(window) {
            return;
        }
        let indices = self.command_matches(cx);
        if indices.is_empty()
            || !matches!(
                key,
                "up" | "down" | "escape" | "tab" | "shift-tab" | "enter"
            )
        {
            return;
        }
        let Some(thread) = self.current() else {
            return;
        };
        let names = indices
            .iter()
            .map(|index| {
                thread.read(cx).configuration().commands()[*index]
                    .name
                    .clone()
            })
            .collect::<Vec<_>>();
        self.composer.command_action(key, &names, window, cx);
        cx.stop_propagation();
        cx.notify();
    }
    pub(super) fn render_commands(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.composer.focus(cx).is_focused(window) {
            return None;
        }
        let commands = self.command_matches(cx);
        if commands.is_empty() {
            return None;
        }
        let scroll = self.composer.command_list_state(&commands);
        let indices = commands;
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
                    cx.processor(move |this, index, _, cx| {
                        this.render_command_row(index, indices[index], cx)
                    }),
                )
                .with_sizing_behavior(ListSizingBehavior::Infer)
                .w_full()
                .max_h(px(192.)),
            );
        Some(div().mx_3().min_w_0().child(menu).into_any_element())
    }
    fn render_command_row(
        &self,
        index: usize,
        command_index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(thread) = self.current() else {
            return div().into_any_element();
        };
        let command = thread.read(cx).configuration().commands()[command_index].clone();
        let origin = CommandOrigin {
            chat: thread.entity_id(),
            advertisement: thread.read(cx).configuration().commands_revision(),
        };
        CommandRenderer::row(
            index,
            &command,
            index == self.composer.selected_command(),
            origin,
            cx,
        )
    }
    fn on_command_selected(
        &mut self,
        origin: CommandOrigin,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.current().is_some_and(|thread| {
            thread.entity_id() == origin.chat
                && thread.read(cx).configuration().commands_revision() == origin.advertisement
        }) {
            self.complete_command(name, window, cx);
        }
    }
}
#[derive(Clone, Copy)]
struct CommandOrigin {
    chat: gpui_kit::EntityId,
    advertisement: u64,
}
struct CommandRenderer;
impl CommandRenderer {
    fn row(
        index: usize,
        command: &acp::AvailableCommand,
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
