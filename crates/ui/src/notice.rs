//! Window-side operational notices, shown as one line in the window's footer
//! ([`NoticeBar`]) that opens into the list of every notice.
//!
//! A notice with an action stays on the line until the user acts on it or
//! dismisses it. Others leave the line after a few seconds and stay in the
//! list until dismissed. Repeating a `key` replaces the earlier notice.

use std::{collections::HashMap, rc::Rc, time::Duration};

use gpui_kit::{App, Global, SharedString, Window, WindowId};

mod bar;
#[cfg(feature = "user-plugins")]
mod plugin;

pub use bar::NoticeBar;
#[cfg(feature = "user-plugins")]
pub use plugin::PluginNotices;

/// How long a notice without an action stays on the footer line.
const TRANSIENT: Duration = Duration::from_secs(8);
/// Notices of the application kept per window; the oldest go first.
const LIMIT: usize = 50;
/// Notices one user plugin keeps in a window, and all user plugins together. They are a separate
/// class: they evict only each other, never a notice of the application.
const PLUGIN_LIMIT: usize = 3;
const PLUGIN_TOTAL: usize = 16;

/// A plugin's own name for one of its notices; repeating it replaces the earlier notice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalKey(String);

impl LocalKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }
}

/// Who a notice belongs to and what it is called. The key is what repeating replaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoticeKey {
    /// A notice of the application.
    Host(&'static str),
    /// A notice a user plugin posted.
    Plugin(nocterm_plugin_model::PluginId, LocalKey),
}

impl NoticeKey {
    fn is_host(&self) -> bool {
        matches!(self, Self::Host(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Success,
    Warning,
    Error,
}

type Action = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone)]
struct Notice {
    id: u64,
    key: NoticeKey,
    level: Level,
    title: SharedString,
    message: SharedString,
    action: Option<(&'static str, Action)>,
    /// Shown on the footer line.
    live: bool,
}

#[derive(Default)]
struct WindowNotices {
    /// Oldest first.
    notices: Vec<Notice>,
    expanded: bool,
}

impl WindowNotices {
    /// Adds `notice` in place of any earlier notice of its key, then evicts the oldest notices of
    /// its own class beyond that class's quota.
    fn insert(&mut self, notice: Notice) {
        self.notices.retain(|earlier| earlier.key != notice.key);
        let key = notice.key.clone();
        self.notices.push(notice);
        match key {
            NoticeKey::Host(_) => self.evict(LIMIT, |key| key.is_host()),
            NoticeKey::Plugin(plugin, _) => {
                self.evict(
                    PLUGIN_LIMIT,
                    |key| matches!(key, NoticeKey::Plugin(owner, _) if *owner == plugin),
                );
                self.evict(PLUGIN_TOTAL, |key| !key.is_host());
            }
        }
    }

    fn evict(&mut self, limit: usize, class: impl Fn(&NoticeKey) -> bool) {
        let members = self.notices.iter().filter(|notice| class(&notice.key));
        let excess = members.clone().count().saturating_sub(limit);
        let doomed: Vec<u64> = members.take(excess).map(|notice| notice.id).collect();
        self.notices.retain(|notice| !doomed.contains(&notice.id));
    }

    /// Whether an action of the application waits on the footer line.
    fn host_action_live(&self) -> bool {
        self.notices
            .iter()
            .any(|notice| notice.live && notice.key.is_host() && notice.action.is_some())
    }
}

#[derive(Default)]
struct Notices {
    windows: HashMap<WindowId, WindowNotices>,
    next: u64,
}
impl Global for Notices {}

fn notices<'a>(window: &Window, cx: &'a mut App) -> &'a mut WindowNotices {
    cx.default_global::<Notices>()
        .windows
        .entry(window.window_handle().window_id())
        .or_default()
}

fn bounded(message: impl Into<SharedString>) -> SharedString {
    let message: SharedString = message.into();
    let mut chars = message.chars();
    let text: String = chars.by_ref().take(512).collect();
    if chars.next().is_some() {
        format!("{text}…").into()
    } else {
        text.into()
    }
}

fn post(
    window: &mut Window,
    cx: &mut App,
    level: Level,
    key: &'static str,
    title: &'static str,
    message: impl Into<SharedString>,
    action: Option<(&'static str, Action)>,
) {
    post_keyed(
        window,
        cx,
        NoticeKey::Host(key),
        level,
        title.into(),
        message.into(),
        action,
    );
}

fn post_keyed(
    window: &mut Window,
    cx: &mut App,
    key: NoticeKey,
    level: Level,
    title: SharedString,
    message: SharedString,
    action: Option<(&'static str, Action)>,
) {
    let id = {
        let store = cx.default_global::<Notices>();
        store.next += 1;
        store.next
    };
    let transient = action.is_none();
    notices(window, cx).insert(Notice {
        id,
        key,
        level,
        title,
        message: bounded(message),
        action,
        live: true,
    });
    window.refresh();
    if transient {
        window
            .spawn(cx, async move |cx| {
                cx.background_executor().timer(TRANSIENT).await;
                let _ = cx.update(|window, cx| {
                    if let Some(notice) = notices(window, cx)
                        .notices
                        .iter_mut()
                        .find(|notice| notice.id == id)
                    {
                        notice.live = false;
                        window.refresh();
                    }
                });
            })
            .detach();
    }
}

fn dismiss(window: &mut Window, cx: &mut App, id: u64) {
    let list = notices(window, cx);
    list.notices.retain(|notice| notice.id != id);
    if list.notices.is_empty() {
        list.expanded = false;
    }
    window.refresh();
}

/// Runs the action of notice `id` and dismisses it.
fn act(window: &mut Window, cx: &mut App, id: u64) {
    let action = notices(window, cx)
        .notices
        .iter()
        .find(|notice| notice.id == id)
        .and_then(|notice| notice.action.clone());
    dismiss(window, cx, id);
    if let Some((_, action)) = action {
        action(window, cx);
    }
}

fn set_expanded(window: &mut Window, cx: &mut App, expanded: bool) {
    let list = notices(window, cx);
    let expanded = expanded && !list.notices.is_empty();
    if list.expanded != expanded {
        list.expanded = expanded;
        window.refresh();
    }
}

/// Removes one keyed operational notice without affecting unrelated notices.
pub fn remove(window: &mut Window, cx: &mut App, key: &'static str) {
    let list = notices(window, cx);
    let before = list.notices.len();
    list.notices
        .retain(|notice| notice.key != NoticeKey::Host(key));
    if list.notices.is_empty() {
        list.expanded = false;
    }
    // A refresh redraws the whole window past every cache: only for a change.
    if list.notices.len() != before {
        window.refresh();
    }
}

/// Removes every notice of the window.
pub fn clear(window: &mut Window, cx: &mut App) {
    let list = notices(window, cx);
    if list.notices.is_empty() && !list.expanded {
        return;
    }
    list.notices.clear();
    list.expanded = false;
    window.refresh();
}

/// How many notices the window keeps, on the footer line or not.
pub fn count(window: &Window, cx: &mut App) -> usize {
    notices(window, cx).notices.len()
}

/// Runs the action of the notice with `key`, as its button does. Returns
/// whether there was one.
pub fn run_action(window: &mut Window, cx: &mut App, key: &'static str) -> bool {
    let id = notices(window, cx)
        .notices
        .iter()
        .find(|notice| notice.key == NoticeKey::Host(key) && notice.action.is_some())
        .map(|notice| notice.id);
    if let Some(id) = id {
        act(window, cx, id);
    }
    id.is_some()
}

pub fn error(
    window: &mut Window,
    cx: &mut App,
    key: &'static str,
    title: &'static str,
    message: impl Into<SharedString>,
) {
    post(window, cx, Level::Error, key, title, message, None);
}

pub fn warning(
    window: &mut Window,
    cx: &mut App,
    key: &'static str,
    title: &'static str,
    message: impl Into<SharedString>,
) {
    post(window, cx, Level::Warning, key, title, message, None);
}

pub fn success(
    window: &mut Window,
    cx: &mut App,
    key: &'static str,
    title: &'static str,
    message: impl Into<SharedString>,
) {
    post(window, cx, Level::Success, key, title, message, None);
}

pub fn info(
    window: &mut Window,
    cx: &mut App,
    key: &'static str,
    title: &'static str,
    message: impl Into<SharedString>,
) {
    post(window, cx, Level::Info, key, title, message, None);
}

/// An actionable error stays visible until the user chooses the recovery or
/// dismisses it. Repeating the same `key` replaces the earlier notice.
pub fn error_action(
    window: &mut Window,
    cx: &mut App,
    key: &'static str,
    title: &'static str,
    message: impl Into<SharedString>,
    action_label: &'static str,
    action: impl Fn(&mut Window, &mut App) + 'static,
) {
    let action: Action = Rc::new(action);
    post(
        window,
        cx,
        Level::Error,
        key,
        title,
        message,
        Some((action_label, action)),
    );
}

pub fn warning_action(
    window: &mut Window,
    cx: &mut App,
    key: &'static str,
    title: &'static str,
    message: impl Into<SharedString>,
    action_label: &'static str,
    action: impl Fn(&mut Window, &mut App) + 'static,
) {
    let action: Action = Rc::new(action);
    post(
        window,
        cx,
        Level::Warning,
        key,
        title,
        message,
        Some((action_label, action)),
    );
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod performance_tests;
