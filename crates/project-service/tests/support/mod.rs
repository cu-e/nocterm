use std::sync::Arc;

use nocterm_project_model::*;
use nocterm_project_service::{PreparedProjects, ProjectsHandle};
use nocterm_store::{ShutdownCoordinator, SnapshotWriter};

pub(crate) fn project(value: u8) -> ProjectId {
    ProjectId::from_bytes([value; 16])
}

pub(crate) fn definition(value: u8) -> ProjectDefinition {
    ProjectDefinition::try_from(ProjectDraft {
        id: project(value),
        name: "Project".into(),
        host: HostLocator::Local,
        root: RootPath::parse("/srv/project").unwrap(),
        source: ProjectSource::Manual,
        updated: 123,
    })
    .unwrap()
}

pub(crate) fn ready_writer() -> SnapshotWriter<Catalog> {
    Arc::new(|_| Box::pin(async { Ok(()) }))
}

pub(crate) fn install(
    prepared: PreparedProjects,
    writer: SnapshotWriter<Catalog>,
    shutdown: &ShutdownCoordinator,
    cx: &mut gpui::TestAppContext,
) -> ProjectsHandle {
    cx.update(|cx| {
        ProjectsHandle::install(
            prepared,
            writer,
            Arc::new(testing::SequenceProjectMetadata::new(17, 123)),
            shutdown,
            cx,
        )
        .unwrap()
    })
}

pub(crate) fn memory(initial: Catalog, cx: &mut gpui::TestAppContext) -> ProjectsHandle {
    install(
        PreparedProjects::loaded(Ok(Some(initial)), false),
        ready_writer(),
        &ShutdownCoordinator::default(),
        cx,
    )
}
