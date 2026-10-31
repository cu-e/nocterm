//! The chats the account's registered runners know, listed through their authenticated
//! catalogs.

mod entry;

use std::sync::Arc;

use nocterm_cloud_agents_model::ChatHistoryEntry;
use nocterm_cloud_model::journal::CurrentChatHead;

use super::error::{self, CloudChatError};
use super::session::OpenedAccount;
use super::{Shared, managed};
use crate::cloud::AccountRunnerCatalog;
use crate::devices::Captured;
use crate::hub_account::HubAccount;
use entry::Entry;

/// Chats listed per hub: the catalog is paged, the menu is not.
const MAX_CHATS: usize = 64;
const MAX_PAGES: usize = 4;

pub(super) async fn list(
    shared: Arc<Shared>,
    captured: Arc<Captured>,
    account: HubAccount,
) -> Result<Vec<Arc<dyn ChatHistoryEntry>>, CloudChatError> {
    let opened = OpenedAccount::open(&account, shared.handle(), shared.budget()).await?;
    let seats = managed::seats(&captured).await?;
    let mut entries: Vec<Arc<dyn ChatHistoryEntry>> = Vec::new();
    for registration in opened.runners()? {
        let catalog = AccountRunnerCatalog::capture(
            opened.material(),
            registration,
            shared.budget().catalog_resident()?,
            shared.handle().clone(),
            shared.collaborators().timer.clone(),
        )
        .map_err(error::catalog)?;
        let runner = *registration.binding().runner().as_bytes();
        let mut page = catalog
            .first(
                shared.budget().catalog_work()?,
                shared.budget().catalog_page()?,
            )
            .await
            .map_err(error::catalog)?;
        for number in 0..MAX_PAGES {
            for head in page.entries().map_err(error::catalog)? {
                if entries.len() >= MAX_CHATS {
                    return Ok(entries);
                }
                if let Some(chat) = chat(head) {
                    entries.push(Arc::new(Entry::new(
                        shared.clone(),
                        captured.clone(),
                        opened.material().clone(),
                        registration,
                        chat,
                        seats.get(&runner).cloned(),
                    )));
                }
            }
            if !page.has_more() || number + 1 == MAX_PAGES {
                break;
            }
            page = catalog
                .next(
                    &page,
                    shared.budget().catalog_work()?,
                    shared.budget().catalog_page()?,
                )
                .await
                .map_err(error::catalog)?;
        }
    }
    Ok(entries)
}

/// A chat the person can open: not deleted, and claimed by its runner.
fn chat(head: &CurrentChatHead) -> Option<&CurrentChatHead> {
    let facts = head.facts();
    (!facts.deleted && facts.writer.is_some()).then_some(head)
}
