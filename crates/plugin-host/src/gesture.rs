//! Single-use proof that the user just acted on a plugin's view.

use std::time::Instant;

use gpui::WindowId;
use nocterm_plugin_api::{Event, LocalId};
use nocterm_plugin_model::PluginId;

use crate::caps::GESTURE_TTL;

/// Where an event came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// A pointer press on a host control.
    Pointer,
    /// Enter or Space on a host control that has focus; `repeat` is set for key repeat.
    Key { repeat: bool },
    /// Anything the host raised itself: visibility, settings, a timer.
    Host,
}

/// Names one minted token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TokenId(u64);

struct Token {
    id: TokenId,
    plugin: PluginId,
    window: WindowId,
    local: LocalId,
    minted: Instant,
}

/// The tokens that are still valid.
#[derive(Default)]
pub(crate) struct Gestures {
    live: Vec<Token>,
    next: u64,
}

impl Gestures {
    /// Mints a token for an activation or toggle by pointer, or by Enter or Space without
    /// repeat. Nothing else mints one.
    pub(crate) fn mint(
        &mut self,
        plugin: &PluginId,
        window: WindowId,
        local: &LocalId,
        event: &Event,
        input: Input,
        now: Instant,
    ) -> Option<TokenId> {
        self.live
            .retain(|token| now.saturating_duration_since(token.minted) < GESTURE_TTL);
        let acted = matches!(event, Event::Activated { .. } | Event::Toggled { .. });
        let by_user = matches!(input, Input::Pointer | Input::Key { repeat: false });
        if !(acted && by_user) {
            return None;
        }
        self.next += 1;
        let id = TokenId(self.next);
        self.live.push(Token {
            id,
            plugin: plugin.clone(),
            window,
            local: local.clone(),
            minted: now,
        });
        Some(id)
    }

    /// Spends the token if it is the plugin's own, from this window and contribution, young
    /// enough and not spent before.
    pub(crate) fn spend(
        &mut self,
        id: Option<TokenId>,
        plugin: &PluginId,
        window: WindowId,
        local: &LocalId,
        now: Instant,
    ) -> bool {
        let Some(id) = id else { return false };
        let found = self.live.iter().position(|token| {
            token.id == id
                && token.plugin == *plugin
                && token.window == window
                && token.local == *local
                && now.saturating_duration_since(token.minted) < GESTURE_TTL
        });
        match found {
            Some(index) => {
                self.live.swap_remove(index);
                true
            }
            None => false,
        }
    }

    pub(crate) fn forget(&mut self, plugin: &PluginId) {
        self.live.retain(|token| token.plugin != *plugin);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn press() -> Event {
        Event::Activated { key: "go".into() }
    }

    fn local(name: &str) -> LocalId {
        LocalId::parse(name).unwrap()
    }

    fn plugin() -> PluginId {
        PluginId::owned("user-acme-clock")
    }

    #[test]
    fn a_token_is_spent_once() {
        let (mut gestures, now) = (Gestures::default(), Instant::now());
        let window = WindowId::default();
        let id = gestures.mint(
            &plugin(),
            window,
            &local("face"),
            &press(),
            Input::Pointer,
            now,
        );
        assert!(id.is_some());
        assert!(gestures.spend(id, &plugin(), window, &local("face"), now));
        assert!(!gestures.spend(id, &plugin(), window, &local("face"), now));
    }

    #[test]
    fn a_token_is_valid_for_less_than_a_second() {
        let (mut gestures, now) = (Gestures::default(), Instant::now());
        let window = WindowId::default();
        let id = gestures.mint(
            &plugin(),
            window,
            &local("face"),
            &press(),
            Input::Pointer,
            now,
        );
        let late = now + GESTURE_TTL;
        assert!(!gestures.spend(id, &plugin(), window, &local("face"), late));

        let id = gestures.mint(
            &plugin(),
            window,
            &local("face"),
            &press(),
            Input::Pointer,
            now,
        );
        let almost = now + GESTURE_TTL - Duration::from_millis(1);
        assert!(gestures.spend(id, &plugin(), window, &local("face"), almost));
    }

    #[test]
    fn a_token_belongs_to_one_plugin_contribution_and_window() {
        let (mut gestures, now) = (Gestures::default(), Instant::now());
        let window = WindowId::default();
        let id = gestures.mint(
            &plugin(),
            window,
            &local("face"),
            &press(),
            Input::Pointer,
            now,
        );
        let other = PluginId::owned("user-acme-other");
        assert!(!gestures.spend(id, &other, window, &local("face"), now));
        assert!(!gestures.spend(id, &plugin(), window, &local("board"), now));
        assert!(gestures.spend(id, &plugin(), window, &local("face"), now));
    }

    #[test]
    fn only_a_press_or_a_toggle_by_the_user_mints() {
        let (mut gestures, now) = (Gestures::default(), Instant::now());
        let window = WindowId::default();
        let mut mint = |event: &Event, input| {
            gestures
                .mint(&plugin(), window, &local("face"), event, input, now)
                .is_some()
        };
        let toggle = Event::Toggled {
            key: "t".into(),
            on: true,
        };
        assert!(mint(&press(), Input::Pointer));
        assert!(mint(&toggle, Input::Key { repeat: false }));
        assert!(!mint(&press(), Input::Key { repeat: true }));
        assert!(!mint(&press(), Input::Host));
        assert!(!mint(&Event::SettingsChanged, Input::Pointer));
        assert!(!mint(
            &Event::VisibilityChanged { visible: true },
            Input::Pointer
        ));
    }

    #[test]
    fn forgetting_a_plugin_drops_its_tokens() {
        let (mut gestures, now) = (Gestures::default(), Instant::now());
        let window = WindowId::default();
        let id = gestures.mint(
            &plugin(),
            window,
            &local("face"),
            &press(),
            Input::Pointer,
            now,
        );
        gestures.forget(&plugin());
        assert!(!gestures.spend(id, &plugin(), window, &local("face"), now));
    }
}
