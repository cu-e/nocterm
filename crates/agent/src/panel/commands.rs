//! ACP command completion and details, with local keyboard navigation.
use super::{
    AgentPanel, MenuKind,
    widgets::{chat_markdown, safe_markdown},
};
use gpui_kit::{
    AnyElement, AnyView, App, Context, Focusable as _, KeyBinding, ListAlignment, ListOffset,
    ListSizingBehavior, ListState, TestSupportExt as _, WeakEntity, Window,
    component::{
        ActiveTheme as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        input::{
            IndentInline, InlineToken, InputContent, MoveDown, MoveUp, OutdentInline, RopeExt as _,
        },
        tooltip::Tooltip,
        v_flex,
    },
    div, list,
    prelude::*,
    px,
};
use nocterm_ai::acp;

pub(super) struct CommandState {
    pub selected: usize,
    pub scroll: ListState,
    visible_matches: Vec<usize>,
    pub dismissed: Option<String>,
    pub cycle: Option<CommandCycle>,
    pub token_prefix: Option<String>,
    pub input_value: String,
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
    pub(super) inserted: String,
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
                .state
                .commands
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
                .state
                .config_options
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
        self.commands.selected = 0;
        self.commands.reveal(0);
        self.commands.cycle = None;
        self.commands.dismissed = None;
        self.commands.token_prefix = None;
        self.commands.input_value.clear();
        self.commands.advertisement = None;
        self.commands.visible_matches.clear();
        self.commands.scroll.reset(0);
    }
    pub(super) fn refresh_commands(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = self.current() else {
            return;
        };
        let key = (thread.entity_id(), thread.read(cx).state.commands_revision);
        if self.commands.advertisement == Some(key) {
            return;
        }
        self.commands.advertisement = Some(key);
        self.commands.visible_matches.clear();
        self.commands.scroll.reset(0);
        self.commands.cycle = None;
        self.commands.selected = 0;
        self.commands.reveal(0);
        self.commands.token_prefix = None;
        let value = self.input.read(cx).value().to_string();
        let removed = self
            .input
            .read(cx)
            .tokens()
            .iter()
            .filter(|span| {
                !thread
                    .read(cx)
                    .state
                    .commands
                    .iter()
                    .any(|command| command.name == span.token().id().as_ref())
            })
            .map(|span| span.range())
            .collect::<Vec<_>>();
        if !removed.is_empty() {
            self.input.update(cx, |input, cx| {
                use gpui_kit::EntityInputHandler as _;
                let selection = input.selected_range();
                for range in removed {
                    let utf16 = value[..range.start].encode_utf16().count()
                        ..value[..range.end].encode_utf16().count();
                    input.replace_text_in_range(Some(utf16), &value[range], window, cx);
                }
                input.set_selected_range(selection, cx);
            });
        }
        self.sync_command_token(window, cx);
        cx.notify();
    }
    pub(super) fn command_matches(&self, cx: &App) -> Vec<usize> {
        let value = self.input.read(cx).value();
        if self.commands.dismissed.as_deref() == Some(value.as_ref()) {
            return Vec::new();
        }
        let filter = self
            .commands
            .cycle
            .as_ref()
            .filter(|cycle| cycle.inserted == value.as_ref())
            .map(|cycle| cycle.filter.as_str())
            .unwrap_or(value.as_ref());
        let Some(prefix) = nocterm_ai::commands::prefix(filter) else {
            return Vec::new();
        };
        let prefix = prefix.to_lowercase();
        self.current()
            .map(|thread| {
                thread
                    .read(cx)
                    .state
                    .commands
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
            })
            .unwrap_or_default()
    }
    /// Annotate only the transition to an advertised command followed by whitespace.
    /// Keeping the prefix after undo avoids immediately recreating an undone token.
    pub(super) fn sync_command_token(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.input.read(cx).value().to_string();
        let invocation = self
            .current()
            .and_then(|thread| {
                nocterm_ai::commands::invocation(&value, &thread.read(cx).state.commands)
            })
            .filter(|invocation| invocation.has_arguments);
        let Some(invocation) = invocation else {
            self.commands.token_prefix = None;
            return;
        };
        let range = invocation.range;
        let name = invocation.name.to_string();
        let prefix = value[range.clone()].to_string();
        if self.commands.token_prefix.as_ref() == Some(&prefix) {
            return;
        }
        self.commands.token_prefix = Some(prefix.clone());
        self.input.update(cx, |input, cx| {
            if input.tokens().iter().any(|span| span.range() == range) {
                return;
            }
            let selection = input.selected_range();
            if input
                .replace_range_with_token(range, InlineToken::new(name, prefix), window, cx)
                .is_ok()
            {
                input.set_selected_range(selection, cx);
            }
        });
    }
    fn complete_command(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let prefix = format!("/{name}");
        let value = format!("{prefix} ");
        self.commands.token_prefix = Some(prefix.clone());
        self.input.update(cx, |input, cx| {
            let content = InputContent::new(value.clone())
                .with_token(0..prefix.len(), InlineToken::new(name, prefix))
                .unwrap_or_else(|_| InputContent::new(value.clone()));
            input.set_value(content, window, cx);
            let end = input.text().offset_to_position(value.len());
            input.set_cursor_position(end, window, cx);
        });
        cx.notify();
    }
    pub(super) fn command_action(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.input.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }
        let commands = self.command_matches(cx);
        if commands.is_empty() {
            return;
        }
        let index = self.commands.selected.min(commands.len() - 1);
        let names = commands
            .iter()
            .map(|index| {
                self.current().unwrap().read(cx).state.commands[*index]
                    .name
                    .clone()
            })
            .collect::<Vec<_>>();
        match key {
            "up" => self.commands.selected = (index + commands.len() - 1) % commands.len(),
            "down" => self.commands.selected = (index + 1) % commands.len(),
            "escape" => {
                self.commands.dismissed = Some(self.input.read(cx).value().to_string());
                self.commands.cycle = None;
            }
            "tab" | "shift-tab" => {
                let cycling = self.commands.cycle.is_some();
                let filter = self
                    .commands
                    .cycle
                    .as_ref()
                    .map(|cycle| cycle.filter.clone())
                    .unwrap_or_else(|| self.input.read(cx).value().to_string());
                let next = if key == "shift-tab" {
                    (index + commands.len() - 1) % commands.len()
                } else if cycling {
                    (index + 1) % commands.len()
                } else {
                    index
                };
                self.commands.selected = next;
                self.commands.cycle = Some(CommandCycle {
                    filter,
                    inserted: format!("/{} ", names[next]),
                });
                self.complete_command(names[next].clone(), window, cx);
            }
            "enter" => {
                self.commands.cycle = None;
                self.complete_command(names[index].clone(), window, cx);
            }
            _ => return,
        }
        self.commands.reveal(self.commands.selected);
        cx.stop_propagation();
        cx.notify();
    }
    pub(super) fn render_commands(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.input.read(cx).focus_handle(cx).is_focused(window) {
            return None;
        }
        let commands = self.command_matches(cx);
        let value = self.input.read(cx).value().to_string();
        if self.commands.dismissed.as_ref() == Some(&value) {
            return None;
        }
        if commands.is_empty() {
            return None;
        }
        if commands != self.commands.visible_matches {
            self.commands.scroll.reset(commands.len());
            self.commands.visible_matches = commands.clone();
            self.commands
                .reveal(self.commands.selected.min(commands.len() - 1));
        }
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
                    self.commands.scroll.clone(),
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
        let command = &thread.read(cx).state.commands[command_index];
        let name = command.name.clone();
        let hover = name.clone();
        let owner = cx.weak_entity();
        div()
            .id(("command-hover", index))
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
                    .when(index == self.commands.selected, |button| {
                        button.bg(cx.theme().accent)
                    })
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
                        this.commands.cycle = None;
                        this.complete_command(name.clone(), window, cx);
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
