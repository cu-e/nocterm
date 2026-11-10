//! What the windows of one installation tell each other about the chats they hold.
//!
//! A chat's document belongs to the one window that holds it; the other windows list it in
//! their history without holding it. Opening it there asks its window to show it and come
//! forward, or, while it neither works nor shows, moves it: the asking window claims it in
//! place of that window's chat, and that window is told to stop listing it.
use gpui::{EntityId, EventEmitter};

/// What the runtime tells every window about the chats windows hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HolderEvent {
    /// A window began to hold a chat.
    Changed,
    /// The window holding chat `chat` shows it and comes forward.
    Reveal { chat: EntityId },
    /// Chat `chat` moved to another window: its window stops listing it.
    Moved { chat: EntityId },
}

impl EventEmitter<HolderEvent> for crate::Runtime {}
