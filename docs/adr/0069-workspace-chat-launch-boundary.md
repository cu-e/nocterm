# 0069. Workspace chat history uses the existing agent lifecycle

Status: accepted

## Decision

Workspace scope is part of chat identity, document ownership, catalog selection
and the original terminal seat. Selecting a workspace shows its chats; switching
tabs does not retarget an existing conversation.

All chats use the existing local ACP connector, configured agent sandbox policy,
provider authentication and terminal sign-in. Workspace membership selects its
ordinary terminal collection. Chats in the same workspace share that collection,
while each document retains its own identity and history.

Chat persistence and normal agent process ownership remain independent of the
folder's terminal tabs.
