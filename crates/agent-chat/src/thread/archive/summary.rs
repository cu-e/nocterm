//! Owns archived metadata without copying an accounted catalog payload.
use nocterm_agent_runtime::CatalogRow;
use nocterm_ai::history::ChatSummary;

pub(in crate::thread) enum ArchivedSummary {
    Owned(Box<ChatSummary>),
    Catalog(CatalogRow),
}
impl From<ChatSummary> for ArchivedSummary {
    fn from(summary: ChatSummary) -> Self {
        Self::Owned(Box::new(summary))
    }
}
impl From<CatalogRow> for ArchivedSummary {
    fn from(row: CatalogRow) -> Self {
        Self::Catalog(row)
    }
}
impl std::ops::Deref for ArchivedSummary {
    type Target = ChatSummary;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Owned(summary) => summary,
            Self::Catalog(row) => row.summary(),
        }
    }
}
