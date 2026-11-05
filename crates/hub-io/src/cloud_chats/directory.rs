//! The chats of the account's runners, listed by the device core's `ListChats` flow: each
//! runner's directory of runner-signed summaries (sealed to the account; replayed from the
//! sealed checkpoint this device keeps) confirmed by the runner's catalog. Rows carry the
//! summary's title, last activity, state, model and bound servers; no chat is replayed.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use nocterm_cloud_agents_model::{
    ChatActivity, ChatFuture, ChatHistoryEntry, ChatListing, ChatSummary, CloudChatError,
    OpenedChat,
};
use nocterm_cloud_conversation_model::CloudConversation as _;
use nocterm_cloud_device::context::ChatRef;
use nocterm_cloud_device::directory::{ListedChat, RowStatus, VerifiedDirectory};
use nocterm_cloud_device::list::{ChatListUpdate, ListChats};
use nocterm_cloud_device::open::ModelChoice;
use nocterm_cloud_device::policy::MANAGED_PROVIDER;
use nocterm_hub_model::cloud::summary::{ChatState, SummaryHost};
use nocterm_hubs_model::HubId;

use super::managed::{self, Seat};
use super::{Shared, error};
use crate::cloud_device::{AccountPlace, ChatSetup, DeviceAccount, directory_file, driver};
use crate::devices::Captured;
use crate::hub_account::HubAccount;

/// Chats listed per hub.
const MAX_CHATS: usize = 256;

pub(super) async fn list(
    shared: &Arc<Shared>,
    captured: Arc<Captured>,
    account: &HubAccount,
    hub: HubId,
) -> Result<Vec<Arc<dyn ChatHistoryEntry>>, CloudChatError> {
    let opened = shared.account(captured.clone(), account).await?;
    let seats = managed::seats(&captured).await?;
    let session = opened.session();
    let place = AccountPlace::of(session.scope());
    let directories: Vec<VerifiedDirectory> = session
        .runners()
        .iter()
        .filter_map(|runner| {
            shared
                .checkpoints
                .load(&place, &directory_file(&runner.runner().id))
                .ok()
                .flatten()
                .map(VerifiedDirectory::from_bytes)
        })
        .collect();
    let device = opened.device().map_err(error::hub)?;
    let mut flow = ListChats::new(session.clone(), device).with_directories(&directories);
    let mut listed: HashMap<[u8; 16], Vec<ListedChat>> = HashMap::new();
    let mut checkpoints = Vec::new();
    driver::run(&mut flow, opened.effects().clone(), |_, update| {
        match update {
            ChatListUpdate::Runner { runner, chats, .. } => {
                listed.insert(runner.runner().id, chats);
            }
            ChatListUpdate::Directory { runner, checkpoint } => {
                checkpoints.push((runner, checkpoint));
            }
            ChatListUpdate::Unreachable { runner, reason } => {
                tracing::debug!(runner = ?runner.runner().id, ?reason, "a runner's chats could not be listed");
            }
            ChatListUpdate::DirectoryRefused { runner, reason } => {
                tracing::warn!(runner = ?runner.runner().id, error = %nocterm_core::error::chain(&reason), "a runner's chat directory was refused");
            }
            ChatListUpdate::Seats(_) => {}
        }
        driver::Control::Continue
    })
    .await
    .map_err(error::device)?;
    save(shared, place, checkpoints);
    let context = Context {
        shared: shared.clone(),
        account: Arc::downgrade(&opened),
        hub,
    };
    let mut entries: Vec<Entry> = Vec::new();
    for registration in session.runners() {
        let runner = registration.runner().id;
        let seat = seats.get(&runner);
        // The chats of a seat that is gone cannot be opened: nothing hosts them any more.
        if seat.is_none() && crate::agents::is_seat_label(registration.label()) {
            continue;
        }
        for chat in listed.remove(&runner).unwrap_or_default() {
            if let Some(entry) = Entry::new(&context, registration.label(), seat, chat) {
                entries.push(entry);
            }
        }
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.summary.updated_at));
    entries.truncate(MAX_CHATS);
    Ok(entries
        .into_iter()
        .map(|entry| Arc::new(entry) as Arc<dyn ChatHistoryEntry>)
        .collect())
}

/// Keeps the runners' verified directories for the next listing.
fn save(shared: &Shared, place: AccountPlace, checkpoints: Vec<([u8; 16], VerifiedDirectory)>) {
    if checkpoints.is_empty() {
        return;
    }
    let files = shared.checkpoints.clone();
    let generation = driver::now_ms().unsigned_abs();
    shared.handle.spawn_blocking(move || {
        for (runner, checkpoint) in checkpoints {
            let stored = files.store(&place, &directory_file(&runner), generation, checkpoint.as_bytes());
            if let Err(error) = stored {
                tracing::warn!(error = %nocterm_core::error::chain(&error), "a chat directory checkpoint was not saved");
            }
        }
    });
}

/// What every row of one listing shares.
#[derive(Clone)]
struct Context {
    shared: Arc<Shared>,
    /// The listing's Cloud session, while a chat of it is open; a row opened later finds the
    /// account again.
    account: Weak<DeviceAccount>,
    hub: HubId,
}

/// One listed chat. Holding it is the only way to open it.
struct Entry {
    context: Context,
    chat: ChatRef,
    setup: ChatSetup,
    summary: ChatSummary,
}

fn activity(state: ChatState) -> Option<ChatActivity> {
    Some(match state {
        ChatState::Idle => ChatActivity::Idle,
        ChatState::Running => ChatActivity::Running,
        ChatState::WaitingApproval => ChatActivity::WaitingApproval,
        ChatState::NeedsGrant => ChatActivity::NeedsGrant,
        ChatState::NeedsProvider => ChatActivity::NeedsProvider,
        ChatState::NeedsSsh => ChatActivity::NeedsSsh,
        ChatState::Failed => ChatActivity::Failed,
        ChatState::Deleted => return None,
    })
}

fn listing(status: RowStatus) -> ChatListing {
    match status {
        RowStatus::Current => ChatListing::Current,
        RowStatus::Stale => ChatListing::Stale,
        RowStatus::AsOf | RowStatus::Unlisted => ChatListing::AsOf,
        RowStatus::Unsummarized | RowStatus::Pending => ChatListing::Unsummarized,
    }
}

/// The row of a chat: the runner's summary when it published one, else the chat's identifier.
fn summary(label: &str, seat: Option<&Seat>, chat: &ListedChat) -> Option<ChatSummary> {
    let key = chat.chat.to_string();
    let short: String = chat.chat.chat_id[..3]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let host = seat.map_or_else(|| label.to_owned(), |seat| seat.name.clone());
    let Some(facts) = chat.summary.as_ref() else {
        let model = seat.map(|seat| seat.model.clone());
        let mut row = ChatSummary::plain(key, format!("Chat · {short}"), host, model);
        row.listing = listing(chat.status);
        return Some(row);
    };
    let activity = Some(activity(facts.state)?);
    let title = if facts.title.is_empty() {
        format!("Chat · {short}")
    } else {
        facts.title.clone()
    };
    Some(ChatSummary {
        key,
        title,
        host,
        model: facts
            .model
            .as_ref()
            .map(|model| model.model.as_str().to_owned())
            .or_else(|| seat.map(|seat| seat.model.clone())),
        updated_at: u64::try_from(facts.last_activity_ms / 1000).ok(),
        activity,
        listing: listing(chat.status),
        servers: facts
            .servers
            .iter()
            .map(|server| format!("{}@{}", server.user.as_str(), server.host.as_str()))
            .collect(),
    })
}

impl Entry {
    fn new(context: &Context, label: &str, seat: Option<&Seat>, chat: ListedChat) -> Option<Self> {
        if chat.deleted {
            return None;
        }
        let summary = summary(label, seat, &chat)?;
        let managed = chat.summary.as_ref().and_then(|facts| match &facts.host {
            SummaryHost::Managed { agent_id } => Some(agent_id.as_str().to_owned()),
            SummaryHost::Own => None,
        });
        // A managed chat prompts its seat's model and wakes the seat for each command; a chat
        // of the person's own runner is history and controls only.
        let wake = seat.map(|seat| seat.agent.to_string()).or(managed);
        let model = wake.as_ref().and_then(|_| {
            summary.model.clone().map(|model| ModelChoice {
                provider: MANAGED_PROVIDER.to_owned(),
                model,
            })
        });
        Some(Self {
            context: context.clone(),
            chat: chat.chat,
            setup: ChatSetup {
                chat: chat.chat,
                model,
                wake,
                servers: None,
            },
            summary,
        })
    }
}

impl ChatHistoryEntry for Entry {
    fn summary(&self) -> &ChatSummary {
        &self.summary
    }

    fn open(&self) -> ChatFuture<OpenedChat> {
        let held = self
            .context
            .account
            .upgrade()
            .filter(|held| held.check().is_ok());
        let setup = self.setup.clone();
        let title = self.summary.title.clone();
        let chat = self.chat;
        let hub = self.context.hub.clone();
        self.context.shared.call(
            &self.context.hub,
            move |shared, captured, account| async move {
                let opened = match held {
                    Some(held) => held,
                    None => shared.account(captured, &account).await?,
                };
                // The listing's runner must still be trusted.
                opened
                    .session()
                    .runner(&chat.runner)
                    .map_err(error::device)?;
                let conversation = shared.conversation(opened, &hub, setup);
                let key = conversation.key().to_string();
                Ok(OpenedChat {
                    conversation,
                    title,
                    key,
                })
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_cloud_device::directory::RowStatus;

    fn listed(status: RowStatus) -> ListedChat {
        let mut chat_id = [0u8; 16];
        chat_id[..3].copy_from_slice(&[0x02, 0xc0, 0x73]);
        ListedChat {
            chat: ChatRef {
                runner: [1; 16],
                chat_id,
                key_epoch: 1,
            },
            summary: None,
            status,
            control_revision: 0,
            head_sequence: None,
            deleted: false,
            witnessed_at_ms: None,
        }
    }

    #[test]
    fn a_chat_without_a_summary_is_named_after_its_identifier_and_keyed_by_its_reference() {
        let row = summary("laptop", None, &listed(RowStatus::Unsummarized)).unwrap();
        assert_eq!(row.title, "Chat · 02c073");
        assert_eq!(row.key, listed(RowStatus::Unsummarized).chat.to_string());
        assert_eq!(row.host, "laptop");
        assert_eq!(row.listing, ChatListing::Unsummarized);
        assert_eq!(row.updated_at, None);
    }

    #[test]
    fn a_row_says_how_far_the_runner_confirmed_it() {
        assert_eq!(listing(RowStatus::Current), ChatListing::Current);
        assert_eq!(listing(RowStatus::AsOf), ChatListing::AsOf);
        assert_eq!(listing(RowStatus::Stale), ChatListing::Stale);
        assert_eq!(activity(ChatState::Deleted), None);
        assert_eq!(
            activity(ChatState::NeedsGrant),
            Some(ChatActivity::NeedsGrant)
        );
    }
}
