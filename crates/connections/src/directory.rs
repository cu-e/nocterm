use gpui_kit::{App, EntityId, WeakEntity, Window};
use nocterm_connections_model::profiles::{Profile, Profiles, Recents};
use nocterm_workspace::{ConnectionDirectory, ConnectionSummary, Workspace};

use std::collections::HashSet;

use crate::{Installed, connect, spec_for_profile};

pub(crate) struct Directory {
    installed: Installed,
}
impl Directory {
    pub(crate) fn new(installed: Installed) -> Self {
        Self { installed }
    }
    fn profile(&self, id: &str, cx: &App) -> Option<Profile> {
        self.installed
            .connections()
            .read(cx)
            .profiles(cx)
            .iter()
            .find(|profile| profile.id.to_string() == id)
            .cloned()
    }
}

impl ConnectionDirectory for Directory {
    fn connections(&self, cx: &App) -> Vec<ConnectionSummary> {
        self.installed
            .connections()
            .read(cx)
            .profiles(cx)
            .iter()
            .map(|profile| summary(profile, &self.installed, cx))
            .collect()
    }

    fn recent_connections(&self, cx: &App) -> Vec<ConnectionSummary> {
        let connections = self.installed.connections().read(cx);
        recent_profiles(connections.profiles(cx), connections.recents(cx))
            .map(|profile| summary(profile, &self.installed, cx))
            .collect()
    }

    fn groups(&self, cx: &App) -> Vec<gpui_kit::SharedString> {
        self.installed
            .connections()
            .read(cx)
            .profiles(cx)
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
        let Some(profile) = self.profile(id, cx) else {
            return false;
        };
        connect(
            &self.installed,
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
        let spec = spec_for_profile(&self.profile(id, cx)?);
        workspace
            .update(cx, |workspace, cx| {
                workspace.open_background_session(spec, window, cx)
            })
            .ok()
            .flatten()
    }
}

fn summary(profile: &Profile, installed: &Installed, cx: &App) -> ConnectionSummary {
    ConnectionSummary {
        id: profile.id.to_string().into(),
        name: profile.name.clone().into(),
        group: profile.group.clone().map(Into::into),
        description: profile.description.clone().into(),
        target: profile.target.clone(),
        icon: installed.icon(profile, cx),
        flag: installed
            .shown_country(profile, cx)
            .and_then(|code| installed.flag(&code, cx)),
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
