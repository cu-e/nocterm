use super::{content::extensions, tools};
use crate::managed::permission::{effect_supported, offer_id_is_valid};
use agent_client_protocol_schema::v1 as wire;
use nocterm_ai::model::retention::RetentionAuthority;
use nocterm_ai::model::{
    auth::*,
    diagnostic::ModelError,
    ids::{AuthMethodId, PermissionOptionId, SessionId},
};

pub(crate) fn method(value: wire::AuthMethod) -> Result<AuthMethod, ModelError> {
    Ok(match value {
        wire::AuthMethod::Agent(value) => AuthMethod {
            id: AuthMethodId::new(value.id.0.as_ref().to_owned()),
            label: value.name,
            description: value.description,
            flow: AuthFlow::AgentManaged,
        },
        wire::AuthMethod::Terminal(value) => AuthMethod {
            id: AuthMethodId::new(value.id.0.as_ref().to_owned()),
            label: value.name,
            description: value.description,
            flow: AuthFlow::TerminalSetup(TerminalSetup {
                args: value.args,
                env: value.env.into_iter().collect(),
            }),
        },
        _ => return Err(ModelError::Unsupported("authentication method")),
    })
}
pub(crate) fn permission(
    authority: &RetentionAuthority,
    value: wire::RequestPermissionRequest,
) -> Result<ProviderPermission, ModelError> {
    Ok(ProviderPermission {
        session: SessionId::new(value.session_id.0.as_ref().to_owned()),
        tool: tools::patch(authority, value.tool_call)?,
        choices: value
            .options
            .into_iter()
            .map(|value| {
                Ok(PermissionChoice {
                    id: PermissionOptionId::new(value.option_id.0.as_ref().to_owned()),
                    label: value.name,
                    effect: match value.kind {
                        wire::PermissionOptionKind::AllowOnce => PermissionEffect::AllowOnce,
                        wire::PermissionOptionKind::AllowAlways => {
                            PermissionEffect::AllowRemembered
                        }
                        wire::PermissionOptionKind::RejectOnce => PermissionEffect::DenyOnce,
                        wire::PermissionOptionKind::RejectAlways => {
                            PermissionEffect::DenyRemembered
                        }
                        _ => PermissionEffect::Unknown,
                    },
                    extensions: extensions(authority, value.meta)?,
                })
            })
            .collect::<Result<_, ModelError>>()?,
        extensions: extensions(authority, value.meta)?,
    })
}
/// Small original-offer authority retained across event ownership transfer.
pub(crate) struct PermissionAuthority {
    choices: Option<std::collections::BTreeMap<PermissionOptionId, PermissionEffect>>,
}
impl PermissionAuthority {
    #[expect(
        clippy::mutable_key_type,
        reason = "identifiers order and hash only by their immutable text"
    )]
    pub(crate) fn original(offer: &ProviderPermission) -> Self {
        let mut choices = std::collections::BTreeMap::new();
        for choice in &offer.choices {
            if !offer_id_is_valid(choice.id.as_str())
                || choices.insert(choice.id.clone(), choice.effect).is_some()
            {
                return Self { choices: None };
            }
        }
        Self {
            choices: Some(choices),
        }
    }
    pub(crate) fn answer(&self, value: PermissionOutcome) -> wire::RequestPermissionOutcome {
        match value {
            PermissionOutcome::Selected(id)
                if self
                    .choices
                    .as_ref()
                    .and_then(|choices| choices.get(&id))
                    .is_some_and(|effect| effect_supported(*effect)) =>
            {
                wire::RequestPermissionOutcome::Selected(wire::SelectedPermissionOutcome::new(
                    wire::PermissionOptionId::new(id.as_str()),
                ))
            }
            _ => wire::RequestPermissionOutcome::Cancelled,
        }
    }
}
pub(super) fn answer(
    offer: &ProviderPermission,
    value: PermissionOutcome,
) -> wire::RequestPermissionOutcome {
    PermissionAuthority::original(offer).answer(value)
}
