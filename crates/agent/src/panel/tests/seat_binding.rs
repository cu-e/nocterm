//! A captured terminal seat belongs to one document and never follows selection.
use super::*;
use nocterm_ai::{execution_origin::ChatKey, history::ContextId, storage::AgentStorageError};
use nocterm_workspace::test_support::ProjectSurfaceProbe as ProjectTab;

#[gpui_kit::test]
fn a_document_seat_rejects_scope_changes_and_rebinding_to_another_chat(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update(|cx| {
        let deps = f._directory.deps(
            f.workspace.downgrade(),
            f.installation.installed.clone(),
            cx,
        );
        let key = ChatKey::new(
            ContextId::General,
            "00000000-0000-0000-0000-000000000001".into(),
        );
        let bound = deps.seat().for_chat(&key).unwrap();
        assert_eq!(bound.id(), f.workspace.entity_id());
        assert!(bound.for_chat(&key).is_ok());
        let foreign = ChatKey::new(
            ContextId::General,
            "00000000-0000-0000-0000-000000000002".into(),
        );
        assert!(matches!(
            bound.for_chat(&foreign),
            Err(AgentStorageError::Codec(
                nocterm_ai::history::HistoryCodecError::IdentityMismatch
            ))
        ));
        let project = serde_json::from_value(
            serde_json::json!({"kind":"project", "id":"01010101010101010101010101010101"}),
        )
        .unwrap();
        assert!(matches!(
            deps.seat()
                .for_chat(&ChatKey::new(project, key.id().into())),
            Err(AgentStorageError::ScopeMismatch { .. })
        ));
        assert_eq!(bound.terminals(cx).unwrap().len(), 1);
    });
}

/// Real outer workspace Item and its ordinary inner terminal surface.
pub(super) fn project_seat(
    f: &Fixture,
    project: nocterm_project_model::ProjectId,
    window: &mut Window,
    cx: &mut App,
) -> (
    nocterm_agent_chat::seat::Seat,
    nocterm_workspace::ProjectSurface,
    Entity<ProjectTab>,
) {
    let (seat, surface, outer) = pending_project_seat(f, project, window, cx);
    f.workspace.update(cx, |workspace, cx| {
        workspace
            .add_project_workspace(outer.clone(), surface.clone(), window, cx)
            .unwrap()
    });
    (seat, surface, outer)
}
pub(super) fn pending_project_seat(
    f: &Fixture,
    project: nocterm_project_model::ProjectId,
    window: &mut Window,
    cx: &mut App,
) -> (
    nocterm_agent_chat::seat::Seat,
    nocterm_workspace::ProjectSurface,
    Entity<ProjectTab>,
) {
    let factory = f.workspace.read(cx).project_surface_factory();
    let outer = cx.new(|cx| ProjectTab::new(factory, project, window, cx).unwrap());
    let surface = outer.read(cx).surface();
    let seat = Rc::new(crate::workspace_seat::WorkspaceSeat::new(
        f.workspace.downgrade(),
        surface.source(cx),
    ));
    (seat, surface, outer)
}

fn terminal_ids(seat: &nocterm_agent_chat::seat::Seat, cx: &App) -> Vec<gpui_kit::EntityId> {
    seat.terminals(cx)
        .unwrap()
        .into_iter()
        .map(|entry| entry.item)
        .collect()
}
#[gpui_kit::test]
fn chats_keep_their_workspace_terminals_while_another_workspace_is_active(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update_window(f.handle, |_, window, cx| {
        let general = f._directory.deps(
            f.workspace.downgrade(),
            f.installation.installed.clone(),
            cx,
        );
        let original_general = terminal_ids(general.seat(), cx);
        let project = nocterm_project_model::ProjectId::from_bytes([1; 16]);
        let (base, surface, _outer) = project_seat(&f, project, window, cx);
        let a = base
            .for_chat(&ChatKey::new(
                ContextId::Project(project),
                "00000000-0000-0000-0000-000000000001".into(),
            ))
            .unwrap();
        let b = base
            .for_chat(&ChatKey::new(
                ContextId::Project(project),
                "00000000-0000-0000-0000-000000000002".into(),
            ))
            .unwrap();
        let mut terminals = Vec::new();
        for _ in 0..2 {
            let terminal = cx.new(|cx| FakeTerminal::new(Access::new(None, Rc::default()), cx));
            surface
                .add_background_item(terminal.clone(), window, cx)
                .unwrap();
            terminals.push(terminal);
        }
        let other_id = nocterm_project_model::ProjectId::from_bytes([2; 16]);
        let (other, other_surface, _other_outer) = project_seat(&f, other_id, window, cx);
        let other = other
            .for_chat(&ChatKey::new(
                ContextId::Project(other_id),
                "00000000-0000-0000-0000-000000000003".into(),
            ))
            .unwrap();
        let foreign = cx.new(|cx| FakeTerminal::new(Access::new(None, Rc::default()), cx));
        other_surface
            .add_background_item(foreign.clone(), window, cx)
            .unwrap();
        let expected: Vec<_> = terminals.iter().map(Entity::entity_id).collect();
        assert_eq!(terminal_ids(&a, cx), expected);
        assert_eq!(terminal_ids(&b, cx), expected);
        assert_eq!(terminal_ids(general.seat(), cx), original_general);
        a.close_background(foreign.entity_id(), window, cx);
        assert_eq!(terminal_ids(&other, cx), [foreign.entity_id()]);
        let own = terminals[1].entity_id();
        let session = nocterm_agent_chat::seat::current_session(&a, own, cx).unwrap();
        assert!(a.close_if_original(own, Some(session), window, cx));
        assert_eq!(terminal_ids(&b, cx), [terminals[0].entity_id()]);
        assert_eq!(
            f.workspace.read(cx).selected_context(cx),
            ContextId::Project(other_id)
        );
    })
    .unwrap();
}
