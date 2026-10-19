//! Original offer authority survives source closure and replacement of UI facts.
use super::{Fixture, fixture};
use crate::managed::{budget::SourceBudget, permission::SelectionAuthority};
use agent_client_protocol::managed::Source;
use nocterm_ai::model::{
    auth::*,
    extensions::Extensions,
    ids::PermissionOptionId,
    json::deferred::literal::LiteralBuilder,
    native::{NativeList, NativeOwned},
};
fn text(value: &str, budget: &SourceBudget) -> NativeOwned<String> {
    let mut builder = LiteralBuilder::new(budget.ledger()).unwrap();
    let mut text = builder.exact_text(value.len()).unwrap();
    text.append(value).unwrap();
    builder.string(text.finish().unwrap()).unwrap();
    NativeOwned::text(builder.finish_native().unwrap().into_text().unwrap())
}
fn offer(
    rules: &[(&str, PermissionEffect)],
    budget: &SourceBudget,
) -> NativeOwned<ProviderPermission> {
    let mut choices = NativeList::new(budget.ledger(), rules.len()).unwrap();
    for (id, effect) in rules {
        choices
            .push(
                nocterm_ai::model::native::auth::permission_choice(
                    nocterm_ai::model::native::auth::permission_option_id(text(id, budget))
                        .unwrap(),
                    text("label", budget),
                    *effect,
                    NativeOwned::<Extensions>::absent(budget.ledger()).unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
    }
    nocterm_ai::model::native::auth::permission(
        nocterm_ai::model::native::controls::session_id(text("original-session", budget)).unwrap(),
        nocterm_ai::model::native::patch::tool_patch(
            nocterm_ai::model::native::tools::tool_id(text("original-tool", budget)).unwrap(),
            None,
            None,
        ),
        choices.finish(),
        NativeOwned::<Extensions>::absent(budget.ledger()).unwrap(),
    )
    .unwrap()
}
#[test]
fn captured_permission_authority_retains_exact_offer_after_source_closure() {
    let Fixture {
        budget,
        _lease: lease,
        _producer: producer,
    } = fixture();
    let offer = offer(
        &[
            (" selected ", PermissionEffect::AllowOnce),
            ("reject", PermissionEffect::DenyOnce),
            ("unknown", PermissionEffect::Unknown),
        ],
        &budget,
    );
    let authority = SelectionAuthority::capture(&offer, budget.clone()).unwrap();
    let selected = PermissionOutcome::Selected(PermissionOptionId::new(" selected "));
    assert_eq!(authority.selection(&selected), Some(" selected "));
    assert_eq!(
        authority.selection(&PermissionOutcome::Selected(PermissionOptionId::new(
            "selected"
        ))),
        None
    );
    assert_eq!(
        authority.selection(&PermissionOutcome::Selected(PermissionOptionId::new(
            "unknown"
        ))),
        None
    );
    assert_eq!(authority.selection(&PermissionOutcome::Cancelled), None);
    drop(offer);
    drop(lease);
    drop(producer);
    assert!(Source::copy("phase is closed", budget).is_err());
    assert_eq!(
        authority.selection(&selected),
        Some(" selected "),
        "captured authority has distinct paid backing through finalawaitdrop"
    );
}
#[test]
fn captured_permission_validation_matches_original_mapper_and_rejects_foreign_source() {
    for rules in [
        vec![
            ("a", PermissionEffect::AllowOnce),
            ("b", PermissionEffect::DenyOnce),
        ],
        vec![
            ("a", PermissionEffect::AllowOnce),
            ("a", PermissionEffect::DenyOnce),
        ],
        vec![(" \n", PermissionEffect::AllowOnce)],
        vec![("a", PermissionEffect::Unknown)],
    ] {
        let fixture = fixture();
        let offer = offer(&rules, &fixture.budget);
        let controlled = SelectionAuthority::capture(&offer, fixture.budget.clone()).unwrap();
        let legacy = crate::mapping::auth::PermissionAuthority::original(offer.get());
        for id in ["a", "b", " \n", "unknown"] {
            let selected = PermissionOutcome::Selected(PermissionOptionId::new(id));
            let actual = controlled.selection(&selected);
            let expected = legacy.answer(selected);
            let expected = serde_json::to_value(expected).unwrap();
            assert_eq!(
                actual.is_some(),
                expected.get("outcome").and_then(serde_json::Value::as_str) == Some("selected"),
                "offer:{rules:?},id:{id:?}"
            );
        }
        let foreign = super::fixture();
        assert!(SelectionAuthority::capture(&offer, foreign.budget).is_err());
    }
}
