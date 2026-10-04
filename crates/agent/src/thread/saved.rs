//! A chat as saved in the history: restoring, forking, naming and saving.
use gpui_kit::{Context, WeakEntity};
use nocterm_ai::{acp, thread::ThreadState};
use nocterm_workspace::Workspace;

use super::{AgentThread, Fork, Restore};
use crate::runtime::Runtime;

impl AgentThread {
    /// A thread showing `chat` from history, connected only when opened.
    pub(crate) fn restored(
        chat: nocterm_ai::history::SavedChat,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut thread = Self::new(chat.agent_id.clone(), workspace, cx);
        thread.chat_id = chat.id;
        thread.updated = chat.updated;
        thread.dormant = true;
        thread.status = "Saved chat".into();
        thread.state.entries = chat.entries;
        thread.state.times = chat.times;
        thread.state.title = chat.title;
        thread.name = chat.name;
        thread.pinned = chat.pinned;
        thread.restore = chat.session_id.map(|session| Restore {
            session: acp::SessionId::new(session),
            workdir: chat.workdir.unwrap_or_default(),
            fork: false,
        });
        thread.last_model = chat.model;
        thread
    }
    /// A copy of this chat, to go on with separately. When opened, the copy
    /// continues in a copy of this chat's agent session.
    pub(crate) fn fork(&self) -> Fork {
        self.fork_at(self.state.entries.len())
    }
    /// A copy of the chat's first `len` entries. Only a copy of the whole
    /// chat continues in a copy of the agent's session; an earlier cut
    /// starts a new session, since the agent cannot forget the rest.
    pub(crate) fn fork_at(&self, len: usize) -> Fork {
        let whole = len >= self.state.entries.len();
        let mut chat = nocterm_ai::history::SavedChat::new(self.agent_id.clone());
        let mut state = self.state.clone();
        state.truncate(len);
        chat.entries = state.entries;
        chat.times = state.times;
        chat.title = self.state.title.clone();
        chat.name = Some(format!("{} (fork)", self.title()));
        chat.model = self.model();
        let restore = match (&self.session, &self.restore) {
            (Some(session), _) => self.session_workdir.clone().map(|workdir| Restore {
                session: session.clone(),
                workdir,
                fork: true,
            }),
            (None, Some(restore)) => Some(Restore {
                fork: true,
                ..restore.clone()
            }),
            (None, None) => None,
        }
        .filter(|_| whole);
        Fork {
            chat,
            restore,
            attachments: self.attachments.clone(),
        }
    }
    /// The chat `fork` made, connected only when opened.
    pub(crate) fn forked(
        fork: Fork,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut thread = Self::restored(fork.chat, workspace, cx);
        thread.restore = fork.restore;
        thread.attachments = fork.attachments;
        thread.status = "Forked chat".into();
        thread
    }
    /// The chat's name: the user's, else the agent's title, else the start of
    /// the first message.
    pub(crate) fn title(&self) -> String {
        chat_title(self.name.as_deref(), &self.state)
    }
    /// The model of the last prompt, in this run or a saved one.
    pub(crate) fn model(&self) -> Option<String> {
        self.last_prompt
            .as_ref()
            .and_then(|prompt| prompt.model.clone())
            .or_else(|| self.last_model.clone())
    }
    pub(crate) fn rename(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        self.name = name
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty());
        self.save(cx);
        cx.notify();
    }
    pub(crate) fn set_pinned(&mut self, pinned: bool, cx: &mut Context<Self>) {
        self.pinned = pinned;
        self.save(cx);
        cx.notify();
    }
    /// What is saved of this chat; `None` until something was said.
    pub(crate) fn snapshot(&self) -> Option<nocterm_ai::history::SavedChat> {
        if self.state.entries.is_empty() {
            return None;
        }
        let mut chat = nocterm_ai::history::SavedChat::new(self.agent_id.clone());
        chat.id = self.chat_id.clone();
        chat.updated = self.updated;
        chat.title = self.state.title.clone();
        chat.name = self.name.clone();
        chat.pinned = self.pinned;
        chat.entries = self.state.entries.clone();
        chat.times = self.state.times.clone();
        chat.model = self.model();
        match (&self.session, &self.restore) {
            (Some(session), _) => {
                chat.session_id = Some(session.0.to_string());
                chat.workdir = self.session_workdir.clone();
            }
            // A fork not yet opened has no session of its own: reopening the
            // original would continue it rather than a copy.
            (None, Some(restore)) if !restore.fork => {
                chat.session_id = Some(restore.session.0.to_string());
                chat.workdir = Some(restore.workdir.clone());
            }
            (None, _) => {}
        }
        Some(chat)
    }
    /// Marks the chat as changed now and saves it.
    pub(crate) fn persist(&mut self, cx: &mut Context<Self>) {
        self.updated = nocterm_ai::history::now();
        self.save(cx);
    }
    /// Saves the chat to history, if anything was said.
    pub(crate) fn save(&mut self, cx: &mut Context<Self>) {
        if let Some(chat) = self.snapshot() {
            // Deferred: this may run while the runtime itself is updating.
            cx.defer(move |cx| {
                Runtime::global(cx).update(cx, |runtime, cx| runtime.save_chat(chat, cx))
            });
        }
    }
}

/// A chat's title: `name`, else the agent's title, else the start of the
/// first message. A chat with nothing in it is a new one.
pub(crate) fn chat_title(name: Option<&str>, state: &ThreadState) -> String {
    if let Some(name) = name.filter(|name| !name.trim().is_empty()) {
        return name.to_owned();
    }
    if state.entries.is_empty() {
        return "New Agent".into();
    }
    state
        .title
        .as_ref()
        .filter(|title| !title.trim().is_empty())
        .cloned()
        .or_else(|| {
            state.entries.iter().find_map(|entry| match entry {
                nocterm_ai::thread::Entry::User(parts) => {
                    parts.iter().find_map(|part| match part {
                        acp::ContentBlock::Text(text) if !text.text.trim().is_empty() => {
                            Some(text.text.trim().chars().take(60).collect())
                        }
                        _ => None,
                    })
                }
                _ => None,
            })
        })
        .unwrap_or_else(|| "New Agent".into())
}
