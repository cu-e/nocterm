# 0032: Terminal consumers use narrow ports and captured session authority

Status: accepted. Supersedes terminal-access and reconnect authority portions of
[0006](0006-workspace-ports.md) and [0011](0011-chat-configuration.md).

## Context

One terminal interface combined screen observation, input, execution, user secrets
and reconnection. Queued approvals and sign-in callbacks could resolve a replacement
session or an identical-looking newer prompt after the user accepted an older card.
The terminal model entity also differs from the workspace Item that owns its view.

## Decision

Workspace defines eight separate contracts: state, text source, input, live commands,
execution, sign-in prompts, reconnect and user paste. Item capabilities contain one
required state handle and seven optional operation handles; missing operations refuse
with typed errors. No composite compatibility trait or shell-input fallback remains.

Every operation holds an ExpectedSession identifying the original workspace Item
and connection epoch. The terminal adapter retains weak Item and model handles and
checks both identity and epoch before observation, executor acquisition or submission.
Sign-in additionally checks a UserPromptToken's prompt epoch and a current Secret
request. The user-only prompt port and Secret never enter agent protocol projections.

Agent intake captures session authority before approvals queue. A changed connection
invalidates a pending card and reusable terminal grants. Intentional reconnect returns
the exact synchronously accepted epoch; each waiter remains bound to that receipt,
its original workspace and agent lifecycle/registration. An old waiter cannot remove
a newer reconnect record or sign-in wait. Background promotion preserves the Item.

Sign-in callbacks capture their original token, port and input entity. Replacement
prompts discard old drafts even when label and masking match. Stale callbacks clear
only their original input and refuse rather than substitute a newer prompt token.

## Alternatives

Keeping a broad alias or composite supertrait would preserve unnecessary capabilities
for every consumer. Resolving current epochs after approval would silently renew
user authority. Using model IDs or output generations would conflate ownership or
screen changes with connection lifetime. Reconnect without a receipt could let a
waiter accept an unrelated later session.

## Consequences

Providers may implement only useful operations. Typed sources remain intact until
notices or tool transcripts. Reads, input, live observation and execution cannot
cross replacement epochs. Terminal and agent coordinators gain no new state fields;
private bounded owners hold queued authority, reconnect receipts and session grants.
