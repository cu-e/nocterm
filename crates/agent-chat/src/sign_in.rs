//! How a chat runs an agent's sign-in command in a terminal.
//!
//! Chats request sign-in only through [`SignInTerminal`]; the window shell
//! supplies the implementation.
use crate::{TerminalAuthError, TerminalAuthRequest};
use futures::channel::oneshot;
use gpui::{App, Window};

pub trait SignInTerminal: 'static {
    /// Opens `request`'s sign-in terminal; resolves when the command ends.
    fn open(
        &self,
        request: TerminalAuthRequest,
        window: &mut Window,
        cx: &mut App,
    ) -> oneshot::Receiver<Result<(), TerminalAuthError>>;
}
