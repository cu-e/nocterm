# 0152. Every window lists every chat, and a chat moves to the window that opens it

Status: accepted. Keeps the rule of [0022](0022-agent-storage-owners.md) and
[0063](0063-agent-chats-below-the-panel.md) that one window holds a chat's document at a time.

## Context

A chat's document has one owner, the window that claimed it (`DocumentRegistry::claim`). The
first window claimed every saved chat when it adopted the catalog, archived placeholders
included, so a second window of the same installation was refused each of them
(`ChatOpenError::AlreadyOpen`), logged a warning per chat on every runtime change, and showed
an empty history. Its own new chats were missing from the first window's history the same way.
Nothing let a person open, from one window, a chat another window held — or see that an agent
was working in it there.

## Decision

1. **Every window lists every chat.** The runtime names the chats of a collection that other
   windows hold (`RuntimeHandle::held_elsewhere`), through the client port, which now gives the
   chat itself (`SessionClient::chat`). The panel lists them in its history among its own, by
   the same order and search, with a mark instead of actions, and observes each through a weak
   handle so that its row follows its title and the spinner of a working agent. Adopting saved
   chats skips them instead of being refused.
2. **Opening one moves it while it rests.** While the chat neither works nor shows in its
   window, this window claims it in place of that window's chat — the replacement a restart
   already uses (`RuntimeHandle::replace_document`): a loaded chat carries over what that window
   had of it (`RestartData`), an archived one is restored from its catalog row. The runtime
   refuses a replacement from another window while the chat works or shows there
   (`RuntimeHandle::may_move`), and on commit tells that window the chat moved
   (`HolderEvent::Moved`), so it stops listing it.
3. **Otherwise its window shows it.** A working or shown chat stays where its tools run: opening
   it asks its window to show it and come forward (`HolderEvent::Reveal`).
4. **Windows hear when one begins to hold a chat** (`HolderEvent::Changed`, on each commit) and
   refresh their list once per burst.

## Alternatives rejected

- **Claim only the chat that is open.** Rows of unclaimed chats would have to come from the
  catalog instead of chat entities, which breaks the rule that a reservation precedes every
  chat entity and the residency accounting built on it.
- **Show one chat entity in two windows.** A chat's tools act on its window's terminals; the
  same chat answered in another window would run them where the person is not looking, and its
  lifetime would no longer follow one window.
- **Always bring the holding window forward.** The first window holds every saved chat, so a
  second window could never open one itself.

## Consequences

- Both windows show the same history; each chat belongs to one window at a time.
- An empty chat (nothing written yet) stays listed only in its own window.
- Closing a window releases its chats as before; the other windows then adopt them.
