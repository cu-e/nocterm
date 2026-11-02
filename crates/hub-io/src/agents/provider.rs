//! The provider kinds of the contract, mapped to the desktop's own.

use nocterm_cloud_agents_model::ProviderKind;
use nocterm_hub_model::agents::ProviderKind as Wire;

/// The contract's kind for the desktop's.
pub(super) fn to_wire(kind: ProviderKind) -> Wire {
    match kind {
        ProviderKind::OpenAiChat => Wire::OpenAiChat,
        ProviderKind::AnthropicMessages => Wire::AnthropicMessages,
    }
}

fn from_wire(kind: Wire) -> ProviderKind {
    match kind {
        Wire::OpenAiChat => ProviderKind::OpenAiChat,
        Wire::AnthropicMessages => ProviderKind::AnthropicMessages,
    }
}

/// The kind a definition or a listing names, in the canonical or the legacy spelling the
/// contract accepts; `None` for a kind this client does not speak.
pub fn parse_provider_kind(text: &str) -> Option<ProviderKind> {
    Wire::parse(text).ok().map(from_wire)
}

/// The canonical text of a listed kind; a kind this client does not know stays as sent.
pub(super) fn canonical(text: &str) -> String {
    parse_provider_kind(text)
        .map_or_else(|| text.to_owned(), |kind| to_wire(kind).as_str().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_kinds_and_the_contracts_agree_on_their_ids() {
        for kind in ProviderKind::ALL {
            assert_eq!(to_wire(kind).as_str(), kind.id());
            assert_eq!(parse_provider_kind(kind.id()), Some(kind));
        }
    }

    #[test]
    fn the_legacy_anthropic_spelling_reads_as_messages_and_lists_canonically() {
        assert_eq!(
            parse_provider_kind("anthropic"),
            Some(ProviderKind::AnthropicMessages)
        );
        assert_eq!(canonical("anthropic"), "anthropic_messages");
        assert_eq!(canonical("mystery"), "mystery");
        assert_eq!(parse_provider_kind("mystery"), None);
    }
}
