//! Constructors of the effects a reply carries. Each is one effect capability the manifest must
//! declare; the host checks the grant, the rate and the target again when it applies the effect.

use alloc::string::String;

use nocterm_plugin_api::{
    Edge, Effect, EffectRequest, Effects, LocalId,
    capability::{
        files::{FileAnswer, FileReply},
        net::{Method, Request},
        notifications::{NoticeTone, Post},
        panels::{SetBadge, SetOpen},
        storage::{Remove, Set},
        terminals::{Bar, Open, Write},
        timers::SetTimer,
    },
};

fn request(effect: Effect) -> EffectRequest {
    EffectRequest { key: None, effect }
}

/// `notifications.post`: a notice the host shows as the plugin's own.
pub fn notify(tone: NoticeTone, text: impl Into<String>) -> EffectRequest {
    request(Effect::NotificationsPost(Post {
        tone,
        text: text.into(),
        notice_key: None,
    }))
}

/// `panels.set-open`: opens or closes one of the plugin's own panels.
pub fn open_panel(local: LocalId, open: bool) -> EffectRequest {
    request(Effect::PanelsSetOpen(SetOpen { local, open }))
}

/// `panels.set-badge`: sets or clears the badge number of one of the plugin's contributions.
pub fn badge(local: LocalId, badge: Option<u16>) -> EffectRequest {
    request(Effect::PanelsSetBadge(SetBadge { local, badge }))
}

/// `storage.set`: stores a value under a key of the plugin's own data.
pub fn store(key: impl Into<String>, value: impl Into<String>) -> EffectRequest {
    request(Effect::StorageSet(Set {
        key: key.into(),
        value: value.into(),
    }))
}

/// `storage.remove`: removes a key of the plugin's own data.
pub fn forget(key: impl Into<String>) -> EffectRequest {
    request(Effect::StorageRemove(Remove { key: key.into() }))
}

/// `net.request`: a request to the web address the user entered in the setting the manifest's
/// `[network]` table names. The answer arrives as an `Event::Response` under `key`.
pub fn request_to(
    key: impl Into<String>,
    method: Method,
    path: impl Into<String>,
    body: Option<String>,
) -> EffectRequest {
    keyed(
        request(Effect::NetRequest(Request {
            method,
            path: path.into(),
            body,
            bytes: false,
        })),
        key,
    )
}

/// `net.request` with `bytes`: `body_base64` is sent as raw bytes, and the answer's body comes
/// back as base64 of the bytes the server sent (at most 160 KiB), so a binary file keeps every
/// byte. The answer arrives as an `Event::Response` under `key`.
pub fn request_bytes(
    key: impl Into<String>,
    method: Method,
    path: impl Into<String>,
    body_base64: Option<String>,
) -> EffectRequest {
    keyed(
        request(Effect::NetRequest(Request {
            method,
            path: path.into(),
            body: body_base64,
            bytes: true,
        })),
        key,
    )
}

/// `timers.set`: an `Event::Timer` under `key` after `after_ms` (one second to one hour).
/// Setting a key that waits replaces it.
pub fn timer(key: impl Into<String>, after_ms: u32) -> EffectRequest {
    keyed(request(Effect::TimersSet(SetTimer { after_ms })), key)
}

/// `terminals.open`: opens the terminal tab `terminal`, or shows it. Lines typed into it arrive
/// as `Event::TerminalLine` at the contribution `local`. Only after a click.
pub fn open_terminal(
    local: LocalId,
    terminal: impl Into<String>,
    title: impl Into<String>,
) -> EffectRequest {
    request(Effect::TerminalsOpen(Open {
        local,
        terminal: terminal.into(),
        title: title.into(),
        files: false,
    }))
}

/// `terminals.open` with `files`: like [`open_terminal`], and the plugin serves the tab's files
/// (it needs `terminals.files`): the file browser and AI chats ask with `Event::FileRequest`,
/// and [`answer_file`] answers.
pub fn open_terminal_with_files(
    local: LocalId,
    terminal: impl Into<String>,
    title: impl Into<String>,
) -> EffectRequest {
    request(Effect::TerminalsOpen(Open {
        local,
        terminal: terminal.into(),
        title: title.into(),
        files: true,
    }))
}

/// `terminals.bar`: puts the plugin's toolbar for its own tabs back on the open terminal tab
/// `terminal` after [`hide_terminal_bar`]. The toolbar is on every such tab by default, along the
/// edge the window's layout gives it: `render` is called for it with `cx.terminal()` naming the
/// terminal, and what the user does in it arrives at `on_event` the same way.
pub fn show_terminal_bar(terminal: impl Into<String>) -> EffectRequest {
    request(Effect::TerminalsBar(Bar {
        terminal: terminal.into(),
        edge: Some(Edge::Top),
    }))
}

/// `terminals.bar` without an edge: takes the plugin's toolbar off the tab `terminal`.
pub fn hide_terminal_bar(terminal: impl Into<String>) -> EffectRequest {
    request(Effect::TerminalsBar(Bar {
        terminal: terminal.into(),
        edge: None,
    }))
}

/// `terminals.files`: the answer to the file request `request` of `terminal`.
pub fn answer_file(terminal: impl Into<String>, request: u32, answer: FileAnswer) -> EffectRequest {
    self::request(Effect::TerminalsFiles(FileReply {
        terminal: terminal.into(),
        request,
        answer,
    }))
}

/// `terminals.write`: text, with `\n` line breaks and colour sequences, for an open terminal.
pub fn write_terminal(terminal: impl Into<String>, text: impl Into<String>) -> EffectRequest {
    request(Effect::TerminalsWrite(Write {
        terminal: terminal.into(),
        text: text.into(),
    }))
}

/// Names an effect, so its outcome comes back under `key` in `Cx::outcomes`.
pub fn keyed(mut request: EffectRequest, key: impl Into<String>) -> EffectRequest {
    request.key = Some(key.into());
    request
}

/// The reply of `on_event`: the given effects. The host refuses a set of more than eight as a
/// whole and applies none of it.
pub fn effects(requests: impl IntoIterator<Item = EffectRequest>) -> Effects {
    Effects(requests.into_iter().collect())
}
