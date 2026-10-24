//! The notices a user plugin posts and the ones it still has, for one window.
//!
//! The host asks for a post after it checked the grant, the rate and the text. The list of the
//! window then keeps the notice in its own class: a plugin keeps a few, all plugins together keep
//! a few more, and none of them ever evicts a notice of the application.
use gpui_kit::{AnyWindowHandle, App};
use nocterm_plugin_api::capability::notifications::{Own, PostNotice};
use nocterm_plugin_api::{
    HostError,
    capability::notifications::{NoticeTone, OwnNotice, OwnNotices, Post},
};
use nocterm_plugin_model::PluginId;
use nocterm_plugin_ports::{Apply, Observe};

use super::{Level, LocalKey, NoticeKey, Notices, post_keyed};

/// The plugin notices of one window.
pub struct PluginNotices {
    window: AnyWindowHandle,
}

impl PluginNotices {
    pub fn new(window: AnyWindowHandle) -> Self {
        Self { window }
    }
}

impl Apply<PostNotice> for PluginNotices {
    fn apply(&self, plugin: &PluginId, request: Post, cx: &mut App) -> Result<(), HostError> {
        let level = match request.tone {
            NoticeTone::Info => Level::Info,
            NoticeTone::Caution => Level::Warning,
        };
        let local = request.notice_key.map_or_else(
            || {
                // A notice without a key is its own notice. A key cannot hold the `#`.
                let store = cx.default_global::<Notices>();
                store.next += 1;
                LocalKey::new(format!("#{}", store.next))
            },
            LocalKey::new,
        );
        let key = NoticeKey::Plugin(plugin.clone(), local);
        let title = format!("Plugin · {plugin}");
        self.window
            .update(cx, move |_, window, cx| {
                post_keyed(
                    window,
                    cx,
                    key,
                    level,
                    title.into(),
                    request.text.into(),
                    None,
                );
            })
            .map_err(|_| HostError::Unavailable)
    }
}

impl Observe<Own> for PluginNotices {
    fn sample(&self, plugin: &PluginId, cx: &App) -> Result<OwnNotices, HostError> {
        let own = cx
            .try_global::<Notices>()
            .and_then(|store| store.windows.get(&self.window.window_id()))
            .into_iter()
            .flat_map(|list| list.notices.iter())
            .filter(|notice| matches!(&notice.key, NoticeKey::Plugin(owner, _) if owner == plugin))
            .map(|notice| OwnNotice {
                tone: match notice.level {
                    Level::Warning | Level::Error => NoticeTone::Caution,
                    Level::Info | Level::Success => NoticeTone::Info,
                },
                text: notice.message.to_string(),
            })
            .collect();
        Ok(OwnNotices(own))
    }
}

#[cfg(test)]
mod tests;
