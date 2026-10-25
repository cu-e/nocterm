//! Constructors of the effects a reply carries. Each is one effect capability the manifest must
//! declare; the host checks the grant, the rate and the target again when it applies the effect.

use alloc::string::String;

use nocterm_plugin_api::{
    Effect, EffectRequest, Effects, LocalId,
    capability::{
        notifications::{NoticeTone, Post},
        panels::{SetBadge, SetOpen},
        storage::{Remove, Set},
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
