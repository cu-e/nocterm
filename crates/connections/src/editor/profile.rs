//! The UI boundary supplies native defaults and the saved profile identity.
use super::FormError;
use nocterm_connections_model::draft::{AuthKind, EditorDefaults, ProfileDraft};
use nocterm_connections_model::profiles::{Profile, ProfileId};
use std::path::Path;
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
    Ok(profile.into_profile(id))
}
