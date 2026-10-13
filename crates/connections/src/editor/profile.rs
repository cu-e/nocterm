//! The UI boundary supplies native defaults and the saved profile identity.
use super::FormError;
use crate::store::{Profile, ProfileId};
use nocterm_connections_model::draft::{AuthKind, EditorDefaults, ProfileDraft};
use std::path::{Path, PathBuf};
pub(super) fn build_profile(
    id: ProfileId,
    fields: &ProfileDraft,
    auth: AuthKind,
    local_user: Option<&str>,
    home: Option<&Path>,
) -> Result<Profile, FormError> {
    let mut draft = fields.clone();
    draft.authentication.kind = auth;
    let defaults = EditorDefaults {
        user: local_user.map(str::to_owned),
        home: home.map(Path::to_path_buf),
    };
    let profile = draft
        .validate(&defaults)
        .map_err(|error| FormError::new(error.field(), error.to_string()))?;
    Ok(Profile {
        id,
        name: profile.name,
        description: profile.description,
        options: profile.options,
        target: profile.target,
        auth: profile.auth,
        group: profile.group,
        credential: profile.credential,
        launch: profile.launch,
        icon: profile.icon,
        icon_color: profile.icon_color,
        country: profile.country,
    })
}
pub(crate) fn local_user() -> Option<String> {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|user| !user.is_empty())
}
pub(super) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}
