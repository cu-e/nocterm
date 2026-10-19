//! Original permission authority owns distinct paid ID copies across UI adoption.
use super::{budget::SourceBudget, error::DecodeError};
use agent_client_protocol::managed::{Buffer, Source};
use nocterm_ai::model::{
    auth::{PermissionEffect, PermissionOutcome, ProviderPermission},
    json::deferred::JsonDecodeError,
    native::NativeOwned,
};
type Error = DecodeError<JsonDecodeError>;
struct Rule {
    id: Source<SourceBudget>,
    effect: PermissionEffect,
}
pub(super) struct SelectionAuthority {
    rules: Buffer<Rule, SourceBudget>,
    valid: bool,
}
impl SelectionAuthority {
    pub(super) fn capture(
        offer: &NativeOwned<ProviderPermission>,
        budget: SourceBudget,
    ) -> Result<Self, Error> {
        if !offer.belongs_to_ledger(budget.ledger()) {
            return Err(JsonDecodeError::ForeignLedger.into());
        }
        let choices = &offer.get().choices;
        let mut rules = Buffer::new(budget.clone());
        rules.reserve(choices.len())?;
        let mut valid = true;
        for choice in choices {
            let text = choice.id.as_str();
            valid &= offer_id_is_valid(text);
            // These copies deliberately have independent physical identities.
            // UI adoption/replacement of the offered original IDs cannot retire
            // this awaited authority's source buffers or authentic header credit.
            let id = Source::copy(text, budget.clone())?;
            rules
                .push(Rule {
                    id,
                    effect: choice.effect,
                })
                .map_err(|(error, _)| error)?;
        }
        // Comparison borrows only original text; no reserve/decoder callbacks.
        rules
            .as_mut_slice()
            .sort_unstable_by(|left, right| left.id.text().cmp(right.id.text()));
        valid &= !rules
            .as_slice()
            .windows(2)
            .any(|pair| pair[0].id.text() == pair[1].id.text());
        Ok(Self { rules, valid })
    }
    /// Only the original captured offer can authorize a reply. The returned
    /// text borrows that snapshot, never the UI's supplied ID or current model.
    pub(super) fn selection(&self, value: &PermissionOutcome) -> Option<&str> {
        let PermissionOutcome::Selected(id) = value else {
            return None;
        };
        if !self.valid {
            return None;
        }
        let index = self
            .rules
            .as_slice()
            .binary_search_by(|rule| rule.id.text().cmp(id.as_str()))
            .ok()?;
        let rule = &self.rules.as_slice()[index];
        effect_supported(rule.effect).then(|| rule.id.text())
    }
}
/// Shared original-offer policy; storage owners choose their actual allocation.
pub(crate) fn offer_id_is_valid(id: &str) -> bool {
    !id.trim().is_empty()
}
pub(crate) fn effect_supported(effect: PermissionEffect) -> bool {
    effect != PermissionEffect::Unknown
}
