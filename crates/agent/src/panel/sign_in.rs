//! Original prompt tokens and native secret inputs have one presentation owner.
use super::AgentPanel;
use crate::thread::AgentThread;
use gpui_kit::{
    App, AppContext as _, Context, Entity, EntityId, Subscription, Window,
    component::input::{InputEvent, InputState},
};
use std::collections::HashMap;
/// The password field of a sign-in card.
pub(super) struct SignInInput {
    input: Entity<InputState>,
    masked: bool,
    token: nocterm_workspace::UserPromptToken,
    _submit: Subscription,
}

#[derive(Clone)]
pub(super) struct SignInSubmission {
    thread: gpui_kit::WeakEntity<AgentThread>,
    item: EntityId,
    token: nocterm_workspace::UserPromptToken,
    input: Entity<InputState>,
    prompts: std::rc::Rc<dyn nocterm_workspace::SignInPrompts>,
}
#[derive(Default)]
pub(super) struct SignInCards {
    inputs: HashMap<EntityId, SignInInput>,
}
impl SignInCards {
    pub(super) fn clear(&mut self) {
        self.inputs.clear();
    }
    pub(super) fn reconcile(&mut self, asking: &[EntityId]) {
        self.inputs.retain(|item, _| asking.contains(item));
    }
    fn submission(
        thread: &Entity<AgentThread>,
        wait: &crate::thread::SignInWait,
        input: Entity<InputState>,
    ) -> SignInSubmission {
        SignInSubmission {
            thread: thread.downgrade(),
            item: wait.item(),
            token: wait.prompt().token,
            input,
            prompts: wait.prompts(),
        }
    }
    pub(super) fn input_for(
        &mut self,
        thread: &Entity<AgentThread>,
        wait: &crate::thread::SignInWait,
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) -> (Entity<InputState>, SignInSubmission) {
        let (item, masked, token) = (wait.item(), wait.prompt().masked, wait.prompt().token);
        if let Some(field) = self.inputs.get(&item)
            && field.masked == masked
            && field.token == token
        {
            return (
                field.input.clone(),
                Self::submission(thread, wait, field.input.clone()),
            );
        }
        if let Some(obsolete) = self.inputs.remove(&item) {
            obsolete
                .input
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(masked)
                .placeholder(if masked { "Password" } else { "Answer" })
        });
        let submission = SignInSubmission {
            thread: thread.downgrade(),
            item,
            token,
            input: input.clone(),
            prompts: wait.prompts(),
        };
        let submit = cx.subscribe_in(&input, window, move |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_sign_in(&submission, window, cx);
            }
        });
        self.inputs.insert(
            item,
            SignInInput {
                input: input.clone(),
                masked,
                token,
                _submit: submit,
            },
        );
        (input.clone(), Self::submission(thread, wait, input))
    }
}
impl SignInSubmission {
    pub(super) fn take_original_answer(&self, window: &mut Window, cx: &mut App) -> Option<String> {
        let answer = self.input.read(cx).value().to_string();
        if answer.is_empty() {
            return None;
        }
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        Some(answer)
    }
    pub(super) fn answer(
        &self,
        answer: String,
        cx: &mut App,
    ) -> Result<(), nocterm_workspace::TerminalError> {
        let current = self.thread.upgrade().is_some_and(|thread| {
            thread
                .read(cx)
                .sign_in_waits()
                .iter()
                .any(|wait| wait.item() == self.item && wait.prompt().token == self.token)
        });
        if current {
            self.prompts
                .answer(self.token, nocterm_session::Secret::new(answer), cx)
        } else {
            Err(nocterm_workspace::TerminalError::StalePrompt)
        }
    }
}
