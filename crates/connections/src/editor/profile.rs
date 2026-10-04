//! Turning the form's text into a profile.
use std::path::{Path, PathBuf};

use nocterm_session::{Auth, DEFAULT_PORT, ShellLaunch, Target};

use super::{AuthKind, Fields, FormError, ValidationField};
use crate::store::{Profile, ProfileId};

/// Turns what was typed into a profile, or says what to fix.
pub(super) fn build_profile(
    id: ProfileId,
    fields: &Fields,
    auth: AuthKind,
    local_user: Option<&str>,
    home: Option<&Path>,
) -> Result<Profile, FormError> {
    let host = fields.host.trim();
    if host.is_empty() {
        return Err(FormError::new(
            ValidationField::Host,
            "Enter a host name or address.",
        ));
    }
    if host.contains(char::is_whitespace) || host.contains('@') {
        return Err(FormError::new(
            ValidationField::Host,
            "The host is just the name or address, as in example.com.",
        ));
    }

    let port = match fields.port.trim() {
        "" => DEFAULT_PORT,
        port => port
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or_else(|| {
                FormError::new(
                    ValidationField::Port,
                    format!("`{port}` is not a port number."),
                )
            })?,
    };

    let user = match fields.user.trim() {
        "" => local_user.map(str::to_owned).ok_or_else(|| {
            FormError::new(ValidationField::User, "Enter the user to sign in as.")
        })?,
        user => user.to_owned(),
    };

    let auth = match auth {
        AuthKind::Auto => Auth::Auto,
        AuthKind::Password => Auth::Password,
        AuthKind::Key => match fields.key_path.trim() {
            "" => {
                return Err(FormError::new(
                    ValidationField::Key,
                    "Choose the private key file to sign in with.",
                ));
            }
            path => Auth::Key {
                path: expand_home(path, home),
            },
        },
    };

    let name = match fields.name.trim() {
        "" => host.to_owned(),
        name => name.to_owned(),
    };
    let group = Some(fields.group.trim())
        .filter(|group| !group.is_empty())
        .map(str::to_owned);

    let credential = match fields.credential.trim() {
        "" => None,
        id => Some(
            id.parse()
                .map_err(|message| FormError::new(ValidationField::Credential, message))?,
        ),
    };
    let launch = if fields.override_launch || !fields.cwd.trim().is_empty() {
        let launch = ShellLaunch {
            program: if fields.override_launch {
                Some(fields.program.trim().to_owned()).filter(|value| !value.is_empty())
            } else {
                None
            },
            args: if fields.override_launch && !fields.args.trim().is_empty() {
                serde_json::from_str(&fields.args).map_err(|_| {
                    FormError::new(
                        ValidationField::Arguments,
                        "Arguments must be a JSON array of strings.",
                    )
                })?
            } else {
                Vec::new()
            },
            cwd: Some(fields.cwd.trim().to_owned()).filter(|value| !value.is_empty()),
            env: if fields.override_launch && !fields.env.trim().is_empty() {
                serde_json::from_str(&fields.env).map_err(|_| {
                    FormError::new(
                        ValidationField::Environment,
                        "Environment must be a JSON object of string values.",
                    )
                })?
            } else {
                Default::default()
            },
            integration: if fields.override_launch {
                fields.launch_integration
            } else {
                true
            },
        };
        launch
            .validate()
            .map_err(|message| FormError::new(ValidationField::Launch, message))?;
        Some(launch)
    } else {
        None
    };

    fields
        .options
        .validate()
        .map_err(|message| FormError::new(ValidationField::Session, message))?;
    let icon = Some(fields.icon.trim())
        .filter(|icon| !icon.is_empty())
        .map(str::to_owned);
    let icon_color = match fields.icon_color.trim() {
        "" => None,
        color => {
            let color = if color.starts_with('#') {
                color.to_owned()
            } else {
                format!("#{color}")
            };
            if !crate::os::is_valid_color(&color) {
                return Err(FormError::new(
                    ValidationField::IconColor,
                    format!(
                        "`{}` is not a colour such as #E95420.",
                        fields.icon_color.trim()
                    ),
                ));
            }
            Some(color.to_ascii_uppercase())
        }
    };
    let country = match fields.country.trim() {
        "" => None,
        code => {
            let lower = code.to_ascii_lowercase();
            if !crate::geo::is_country_code(&lower) {
                return Err(FormError::new(
                    ValidationField::Country,
                    format!("`{code}` is not a two-letter country code such as DE."),
                ));
            }
            Some(lower)
        }
    };
    if fields.description.len() > 16384 {
        return Err(FormError::new(
            ValidationField::Description,
            "Description must be at most 16384 bytes.",
        ));
    }
    Ok(Profile {
        description: fields.description.trim().to_owned(),
        options: fields.options.clone(),
        id,
        name,
        target: Target::new(user, host, port),
        auth,
        group,
        credential,
        launch,
        icon,
        icon_color,
        country,
    })
}

/// `~/x` relative to the home directory, as a shell would read it.
pub(super) fn expand_home(path: &str, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

/// The user name to sign in as when none is given.
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
