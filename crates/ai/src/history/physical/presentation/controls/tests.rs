//! Actual published facts retain independent component payloads through replacement.
use crate::{
    history::{
        MAX_RESIDENT_BYTES,
        physical::{ConfigKindSlot, PresentationUpdate, SelectChoicesSlot, SnapshotBuilder},
    },
    model::{
        controls::ConfigOption,
        json::deferred::DecodeLedger,
        native::{NativeList, NativeOwned},
        retention::{
            RetentionAuthority,
            document::{
                ConstructionRole, DocumentRetention, RetainedRole,
                tests::{snapshot, text},
            },
        },
    },
};

fn configuration(ledger: &DecodeLedger, name: &str) -> NativeOwned<Vec<ConfigOption>> {
    let choice = crate::model::native::controls::config_choice(
        crate::model::native::controls::config_choice_id(text(ledger, "choice")).unwrap(),
        text(ledger, "Actual choice"),
        NativeOwned::absent(ledger).unwrap(),
        NativeOwned::absent(ledger).unwrap(),
    )
    .unwrap();
    let mut choices = NativeList::new(ledger, 1).unwrap();
    choices.push(choice).unwrap();
    let group = crate::model::native::controls::config_group(
        crate::model::native::controls::config_group_id(text(ledger, "group")).unwrap(),
        text(ledger, "Actual group"),
        choices.finish(),
        NativeOwned::absent(ledger).unwrap(),
    )
    .unwrap();
    let mut groups = NativeList::new(ledger, 1).unwrap();
    groups.push(group).unwrap();
    let kind = crate::model::native::controls::select(
        crate::model::native::controls::config_choice_id(text(ledger, "choice")).unwrap(),
        crate::model::native::controls::groups(groups.finish()),
    )
    .unwrap();
    let option = crate::model::native::controls::config_option(
        crate::model::native::controls::config_id(text(ledger, "setting")).unwrap(),
        text(ledger, name),
        NativeOwned::absent(ledger).unwrap(),
        NativeOwned::absent(ledger).unwrap(),
        kind,
        NativeOwned::absent(ledger).unwrap(),
    )
    .unwrap();
    let mut options = NativeList::new(ledger, 1).unwrap();
    options.push(option).unwrap();
    options.finish()
}
fn install(role: &mut ConstructionRole, live: &mut RetainedRole, update: PresentationUpdate) {
    let mut builder = SnapshotBuilder::from_role(role, live).unwrap();
    builder.update_presentation(update).unwrap();
    live.replace(builder.finish(None).unwrap()).unwrap();
}

fn commands(ledger: &DecodeLedger, name: &str) -> PresentationUpdate {
    let command = crate::model::native::notice::command(
        text(ledger, name),
        text(ledger, "description"),
        NativeOwned::absent(ledger).unwrap(),
    )
    .unwrap();
    let mut commands = NativeList::new(ledger, 1).unwrap();
    commands.push(command).unwrap();
    PresentationUpdate::Commands(commands.finish())
}
fn assert_grouped_setting(option: &super::ConfigOptionSlot, name: &str) {
    assert_eq!(option.id().as_str(), "setting");
    assert_eq!(&*option.name(), name);
    let ConfigKindSlot::Select {
        current,
        choices: SelectChoicesSlot::Groups(groups),
    } = option.kind()
    else {
        panic!("select groups");
    };
    assert_eq!(current.read().as_str(), "choice");
    assert_eq!(groups[0].id().as_str(), "group");
    assert_eq!(&*groups[0].name(), "Actual group");
    assert_eq!(groups[0].choices()[0].id().as_str(), "choice");
    assert_eq!(&*groups[0].choices()[0].name(), "Actual choice");
}

#[test]
fn configuration_and_mode_updates_preserve_original_commands_and_nested_choices() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut reader = roles.take_reader().unwrap();
    let mut live = roles.take_live().unwrap();
    let mut active = roles.take_writer_active().unwrap();
    let account_baseline = scope.retained_and_pending_bytes().unwrap();
    live.replace(snapshot(&reader, "seed")).unwrap();
    let ledger = DecodeLedger::for_operation(&reader, MAX_RESIDENT_BYTES).unwrap();
    let ledger_baseline = ledger.live_requested_bytes();
    let command_name = "actual-command".repeat(4096);
    install(&mut reader, &mut live, commands(&ledger, &command_name));
    let pointer = live
        .snapshot()
        .unwrap()
        .presentation()
        .unwrap()
        .controls()
        .unwrap()
        .commands()[0]
        .name()
        .as_ptr();
    install(
        &mut reader,
        &mut live,
        PresentationUpdate::Configuration(configuration(&ledger, "Initial setting")),
    );
    active
        .replace(
            SnapshotBuilder::from_role(&mut reader, &live)
                .unwrap()
                .finish(None)
                .unwrap(),
        )
        .unwrap();
    install(
        &mut reader,
        &mut live,
        PresentationUpdate::Mode(
            crate::model::native::controls::mode_id(text(&ledger, "mode")).unwrap(),
        ),
    );
    install(
        &mut reader,
        &mut live,
        PresentationUpdate::Configuration(configuration(&ledger, "Updated setting")),
    );
    let controls = live
        .snapshot()
        .unwrap()
        .presentation()
        .unwrap()
        .controls()
        .unwrap();
    assert_eq!(controls.commands_revision(), 1);
    assert_eq!(controls.commands()[0].name().as_ptr(), pointer);
    assert_eq!(&*controls.commands()[0].name(), command_name);
    assert_eq!(controls.mode().unwrap().as_str(), "mode");
    assert_grouped_setting(&controls.configuration()[0], "Updated setting");
    assert_eq!(
        &*active
            .snapshot()
            .unwrap()
            .presentation()
            .unwrap()
            .controls()
            .unwrap()
            .configuration()[0]
            .name(),
        "Initial setting"
    );
    active.clear();
    live.clear();
    reader.clear_source();
    assert_eq!(ledger.live_requested_bytes(), ledger_baseline);
    drop(ledger);
    assert_eq!(
        scope.retained_and_pending_bytes().unwrap(),
        account_baseline
    );
}
