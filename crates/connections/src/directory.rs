use gpui_kit::{App, EntityId, WeakEntity, Window};
use nocterm_workspace::{ConnectionDirectory, ConnectionSummary, Workspace};

use std::collections::HashSet;

use crate::{
    Connections, ServerFacts, connect, spec_for_profile,
    store::{Profile, Profiles, Recents},
};

pub(crate) struct Directory;

impl ConnectionDirectory for Directory {
    fn connections(&self, cx: &App) -> Vec<ConnectionSummary> {
        let facts = ServerFacts::global(cx).read(cx);
        Connections::global(cx)
            .read(cx)
            .profiles()
            .iter()
            .map(|profile| summary(profile, facts, cx))
            .collect()
    }

    fn recent_connections(&self, cx: &App) -> Vec<ConnectionSummary> {
        let connections = Connections::global(cx).read(cx);
        let facts = ServerFacts::global(cx).read(cx);
        recent_profiles(connections.profiles(), connections.recents())
            .map(|profile| summary(profile, facts, cx))
            .collect()
    }

    fn groups(&self, cx: &App) -> Vec<gpui_kit::SharedString> {
        Connections::global(cx)
            .read(cx)
            .profiles()
            .groups()
            .into_iter()
            .map(|group| group.to_owned().into())
            .collect()
    }

    fn open(
        &self,
        id: &str,
        workspace: &WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some(profile) = profile(id, cx) else {
            return false;
        };
        connect(
            workspace,
            spec_for_profile(&profile),
            Some(profile.id),
            window,
            cx,
        );
        true
    }

    fn open_background(
        &self,
        id: &str,
        workspace: &WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<EntityId> {
        let spec = spec_for_profile(&profile(id, cx)?);
        workspace
            .update(cx, |workspace, cx| {
                workspace.open_background_session(spec, window, cx)
            })
            .ok()
            .flatten()
    }
}

fn summary(profile: &Profile, facts: &ServerFacts, cx: &App) -> ConnectionSummary {
    ConnectionSummary {
        id: profile.id.to_string().into(),
        name: profile.name.clone().into(),
        group: profile.group.clone().map(Into::into),
        description: profile.description.clone().into(),
        target: profile.target.clone(),
        icon: facts.icon(profile),
        flag: facts
            .shown_country(profile, cx)
            .and_then(|code| facts.flag(code)),
    }
}

fn recent_profiles<'a>(
    profiles: &'a Profiles,
    recents: &'a Recents,
) -> impl Iterator<Item = &'a Profile> {
    let mut seen = HashSet::new();
    recents
        .iter()
        .filter_map(move |recent| recent.profile.filter(|id| seen.insert(*id)))
        .filter_map(|id| profiles.get(id))
}

#[cfg(test)]
mod tests;

fn profile(id: &str, cx: &App) -> Option<Profile> {
    Connections::global(cx)
        .read(cx)
        .profiles()
        .iter()
        .find(|profile| profile.id.to_string() == id)
        .cloned()
}
