//! The original background sign-in prompt and its immutable authority facts.
use gpui_kit::EntityId;

/// A background session asks for a secret. The user answers in the chat, or
/// unlocks the vault when a saved secret answers it; no tab opens.
#[derive(Clone)]
pub(crate) struct SignInWait {
    item: EntityId,
    title: gpui_kit::SharedString,
    prompts: std::rc::Rc<dyn nocterm_workspace_ports::SignInPrompts>,
    prompt: nocterm_workspace_ports::SignInPrompt,
    /// Unlocking the vault answers the prompt with a saved secret.
    vault: bool,
}
impl SignInWait {
    pub(crate) fn item(&self) -> EntityId {
        self.item
    }
    pub(crate) fn title(&self) -> &gpui_kit::SharedString {
        &self.title
    }
    pub(crate) fn prompt(&self) -> &nocterm_workspace_ports::SignInPrompt {
        &self.prompt
    }
    pub(crate) fn vault(&self) -> bool {
        self.vault
    }

    pub(crate) fn new(
        item: EntityId,
        title: gpui_kit::SharedString,
        prompts: std::rc::Rc<dyn nocterm_workspace_ports::SignInPrompts>,
        prompt: nocterm_workspace_ports::SignInPrompt,
        vault: bool,
    ) -> Self {
        Self {
            item,
            title,
            prompts,
            prompt,
            vault,
        }
    }
    pub(crate) fn prompts(&self) -> std::rc::Rc<dyn nocterm_workspace_ports::SignInPrompts> {
        self.prompts.clone()
    }
}
