use nocterm_project_model::*;

pub(crate) fn project(value: u8) -> ProjectId {
    ProjectId::from_bytes([value; 16])
}

pub(crate) fn definition(value: u8) -> ProjectDefinition {
    ProjectDraft {
        id: project(value),
        name: "Example".into(),
        target: ProjectTarget::HostFolder {
            host: HostLocator::Local,
            root: RootPath::parse("/home/user/project").unwrap(),
        },
        source: ProjectSource::Manual,
        updated: 123,
    }
    .try_into()
    .unwrap()
}
