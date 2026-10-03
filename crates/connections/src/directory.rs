use gpui_kit::{App, Context, Window};
use nocterm_workspace::{ConnectionDirectory, ConnectionSummary, Workspace};

use crate::{Connections, connect, spec_for_profile};

pub(crate) struct Directory;

impl ConnectionDirectory for Directory {
    fn connections(&self, cx: &App) -> Vec<ConnectionSummary> {
        Connections::global(cx)
            .read(cx)
            .profiles()
            .iter()
            .map(|profile| ConnectionSummary {
                id: profile.id.to_string().into(),
                name: profile.name.clone().into(),
                group: profile.group.clone().map(Into::into),
                description: profile.description.clone().into(),
                target: profile.target.clone(),
            })
            .collect()
    }

    fn open(&self, id: &str, window: &mut Window, cx: &mut Context<Workspace>) -> bool {
        let entity = Connections::global(cx);
        let Some(profile) = entity
            .read(cx)
            .profiles()
            .iter()
            .find(|profile| profile.id.to_string() == id)
            .cloned()
        else {
            return false;
        };
        connect(
            &cx.entity().downgrade(),
            spec_for_profile(&profile),
            Some(profile.id),
            window,
            cx,
        );
        true
    }
}
