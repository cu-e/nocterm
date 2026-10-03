//! The capability gate for everything AI.
//!
//! `ai.enabled` in the settings is the single source of truth. A feature
//! asks [`ActiveAi::ai_enabled`] before it starts anything, and uses
//! [`observe_ai_enabled`] to stop what it started when the switch goes off.

use gpui_kit::{App, Subscription};

use crate::{ActiveSettings as _, SettingsStore};

/// Whether AI features are switched on.
pub trait ActiveAi {
    fn ai_enabled(&self) -> bool;
}

impl ActiveAi for App {
    fn ai_enabled(&self) -> bool {
        self.settings().ai.enabled
    }
}

/// Calls `on_change` with the new state whenever the master switch flips.
///
/// Other settings changes do not call it, and neither does subscribing: read
/// [`ActiveAi::ai_enabled`] for the state at that moment. The observation ends
/// when the returned subscription is dropped.
pub fn observe_ai_enabled(
    cx: &mut App,
    mut on_change: impl FnMut(bool, &mut App) + 'static,
) -> Subscription {
    let mut enabled = cx.ai_enabled();
    cx.observe_global::<SettingsStore>(move |cx| {
        let now = cx.ai_enabled();
        if now != enabled {
            enabled = now;
            on_change(now, cx);
        }
    })
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use gpui_kit::TestAppContext;
    use nocterm_settings::Settings;

    use crate::{SettingsStore, update_settings};

    use super::*;

    fn install(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(SettingsStore::in_memory(Settings::default()));
            crate::settings::init(cx);
        });
    }

    #[gpui_kit::test]
    async fn reflects_the_master_switch(cx: &mut TestAppContext) {
        install(cx);
        cx.update(|cx| assert!(cx.ai_enabled()));

        cx.update(|cx| update_settings(cx, |settings| settings.ai.enabled = false))
            .await
            .unwrap();

        cx.update(|cx| assert!(!cx.ai_enabled()));
    }

    #[gpui_kit::test]
    async fn observers_hear_only_flips(cx: &mut TestAppContext) {
        install(cx);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let log = seen.clone();
        let subscription = cx
            .update(|cx| observe_ai_enabled(cx, move |enabled, _| log.borrow_mut().push(enabled)));

        cx.update(|cx| update_settings(cx, |settings| settings.terminal.copy_on_select = true))
            .await
            .unwrap();
        assert!(seen.borrow().is_empty());

        cx.update(|cx| update_settings(cx, |settings| settings.ai.enabled = false))
            .await
            .unwrap();
        cx.update(|cx| {
            update_settings(cx, |settings| settings.ai.default_agent = Some("x".into()))
        })
        .await
        .unwrap();
        cx.update(|cx| update_settings(cx, |settings| settings.ai.enabled = true))
            .await
            .unwrap();
        assert_eq!(*seen.borrow(), [false, true]);

        drop(subscription);
        cx.update(|cx| update_settings(cx, |settings| settings.ai.enabled = false))
            .await
            .unwrap();
        assert_eq!(*seen.borrow(), [false, true]);
    }
}
