//! The pool keys this device trusts on a server, each with a button that forgets it so the next
//! start on that pool asks for the fingerprint comparison again.

use gpui_kit::{
    App, ClickEvent, Entity, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _,
        button::Button,
        h_flex, v_flex,
    },
    div,
    prelude::*,
};
use nocterm_cloud_agents_model::PinnedPool;

use super::CloudAgentsTab;
use crate::layout::{field, note};
use crate::servers::item;

pub(super) fn pinned_section(
    pinned: &[PinnedPool],
    key: &str,
    view: &Entity<CloudAgentsTab>,
    cx: &App,
) -> Option<impl IntoElement> {
    if pinned.is_empty() {
        return None;
    }
    let mono = cx.theme().mono_font_family.clone();
    let rows = pinned.iter().map(|entry| {
        let (view, pool) = (view.clone(), entry.pool.clone());
        let id = entry.pool.as_str().to_owned();
        h_flex()
            .id(item(&format!("cloud-agents-pinned-{id}"), key))
            .test_support()
            .gap_2()
            .child(div().child(id.clone()))
            .child(
                div()
                    .font_family(mono.clone())
                    .text_sm()
                    .child(entry.fingerprint.clone()),
            )
            .child(
                Button::new(item(&format!("cloud-agents-forget-{id}"), key))
                    .label("Forget")
                    .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        view.update(cx, |tab, cx| tab.forget_pool(&pool, cx));
                    }),
            )
    });
    Some(
        v_flex()
            .id(item("cloud-agents-pinned", key))
            .test_support()
            .gap_2()
            .child(note(
                "Pools trusted on this device. Forgetting one asks you to compare its fingerprint again on the next start.",
                cx,
            ))
            .child(field("Trusted pools", v_flex().gap_1().children(rows))),
    )
}
