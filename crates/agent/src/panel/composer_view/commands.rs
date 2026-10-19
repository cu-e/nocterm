//! Inline command annotations belong to the actual composer input.
use super::ComposerView;
use crate::panel::AgentPanel;
use gpui_kit::{
    App, Context, EntityId, Focusable as _, ListState, Window,
    component::input::{InlineToken, InputContent, RopeExt as _},
};
use nocterm_ai::model::commands::AvailableCommand;
impl ComposerView {
    pub(in crate::panel) fn select_command_token(
        &self,
        range: std::ops::Range<usize>,
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) {
        self.input.update(cx, |input, cx| {
            input.set_selected_range(range, cx);
            window.focus(&input.focus_handle(cx), cx);
        });
    }
    pub(in crate::panel) fn observe_input(&mut self, cx: &App) -> bool {
        self.commands.observe_input(self.text(cx).to_string())
    }
    pub(in crate::panel) fn reset_commands(&mut self) {
        self.commands.reset();
    }
    pub(in crate::panel) fn command_matches(
        &self,
        commands: &[AvailableCommand],
        cx: &App,
    ) -> Vec<usize> {
        self.commands.matches(self.text(cx).as_ref(), commands)
    }
    pub(in crate::panel) fn selected_command(&self) -> usize {
        self.commands.selected()
    }
    pub(in crate::panel) fn command_list_state(&mut self, matches: &[usize]) -> ListState {
        self.commands.list_state(matches)
    }
    pub(in crate::panel) fn refresh_commands(
        &mut self,
        key: (EntityId, u64),
        commands: &[AvailableCommand],
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) -> bool {
        if !self.commands.observe_advertisement(key) {
            return false;
        }
        let value = self.text(cx).to_string();
        let removed = self
            .input
            .read(cx)
            .tokens()
            .iter()
            .filter(|span| {
                !commands
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
        self.sync_command_token(commands, window, cx);
        true
    }
    pub(in crate::panel) fn sync_command_token(
        &mut self,
        commands: &[AvailableCommand],
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) {
        let value = self.text(cx).to_string();
        let invocation = nocterm_ai::commands::invocation(&value, commands)
            .filter(|invocation| invocation.has_arguments);
        let Some(invocation) = invocation else {
            self.commands.observe_token(None);
            return;
        };
        let range = invocation.range;
        let name = invocation.name.to_string();
        let prefix = value[range.clone()].to_string();
        if !self.commands.observe_token(Some(prefix.clone())) {
            return;
        }
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
    pub(in crate::panel) fn complete_command(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) {
        let prefix = format!("/{name}");
        let value = format!("{prefix} ");
        self.commands.observe_token(Some(prefix.clone()));
        self.input.update(cx, |input, cx| {
            let content = InputContent::new(value.clone())
                .with_token(0..prefix.len(), InlineToken::new(name, prefix))
                .unwrap_or_else(|_| InputContent::new(value.clone()));
            input.set_value(content, window, cx);
            let end = input.text().offset_to_position(value.len());
            input.set_cursor_position(end, window, cx);
        });
    }
    pub(in crate::panel) fn confirm_command(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) {
        self.commands.confirm();
        self.complete_command(name, window, cx);
    }
    pub(in crate::panel) fn command_action(
        &mut self,
        key: &str,
        names: &[String],
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) {
        let value = self.text(cx);
        if let Some(name) = self.commands.action(key, value.as_ref(), names) {
            self.complete_command(name, window, cx);
        }
    }
}
