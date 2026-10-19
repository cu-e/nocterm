//! Cleanup metadata lives in its first genuine JSON root identity allocation.
//! The workspace retains that lease, while the lease never retains the workspace.
pub(in crate::model::json) type CleanupIdentity = super::super::lifetime::HeaderLease;
