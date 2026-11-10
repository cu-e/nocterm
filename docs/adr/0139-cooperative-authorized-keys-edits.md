# 0139. Authorized-keys edits use a cooperating remote lock

Status: accepted

Refines the automatic host-edit contract of [ADR0119](0119-agent-ssh-ca.md).

## Context

SFTP atomic replacement does not compare the destination's current bytes. A comparison
before uploading a staged file could therefore lose a second session's edit before rename.
A process-local mutex cannot coordinate another device or another SFTP session.

## Decision

The SSH adapter acquires `~/.ssh/.nocterm-authorized_keys.lock` through the existing
private, no-clobber Create upload protocol before comparing or changing `authorized_keys`.
A random owner token identifies the successful acquisition. All Nocterm sessions and devices
use that fixed path. A second owner receives typed `EditLocked`, preserving the actual path
through the neutral host-access error and the Servers page.

The owner distinguishes a missing file from an existing empty file, preserves unrelated
lines, and compares the observed bytes again immediately before publication. Ordinary
completion checks the lock's bounded token before removing it. A postcommit cleanup failure
cannot report the already-written edit as an apparent precommit refusal.

Interrupted or cancelled work can leave an orphan lock. Nocterm never expires or steals one:
manual recovery requires stopping all edits before removing the stated lock file. Unsupported
no-clobber publication or removal returns a typed error with its native cause. The existing
Copy CA line action provides the manual alternative.

## Limits and alternatives

The protocol coordinates participating Nocterm editors. An arbitrary external editor can
still race the final comparison; neither the port nor UI claims a filesystem compare-and-swap.
Silently stealing an old lock would allow a delayed owner to overwrite its replacement.
Remote `flock` would require another execution capability and a longer-lived helper protocol;
SFTP already supplies the required no-clobber admission primitive.

## Consequences

Concurrent participating edits fail visibly instead of losing an earlier publication.
Cancellation favors preserving access rules over automatic orphan recovery. This adds no
remote executable, generic filesystem contract, retry timer or process-wide registry.
