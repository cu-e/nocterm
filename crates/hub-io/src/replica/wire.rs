//! What a profile object holds and how an object of the feed is read.
//!
//! The plaintext is `{"v":1,"profile":…}`. An object this client cannot read (a newer format
//! version, another kind, or a profile under another id) is reported as unrecognized so the
//! reconciliation leaves it alone. An object that does not authenticate is the hub's
//! forgery: it is an error and the feed does not move past it.

use std::str::FromStr as _;

use nocterm_connections_model::profiles::{Profile, ProfileId};
use nocterm_connections_model::replica::{
    RemoteBody, RemoteProfile, ReplicaCursor, ReplicaError, ReplicaScope, ReplicaVersion,
};
use nocterm_core::SharedCause;
use nocterm_hub_proto::v2;
use nocterm_keys::CollectionKey;
use serde::{Deserialize, Serialize};

/// The kind of the objects that hold profiles.
pub(super) const KIND: &str = "profile";
const FORMAT: u32 = 1;

#[derive(Serialize)]
struct Write<'a> {
    v: u32,
    profile: &'a Profile,
}

#[derive(Deserialize)]
struct Head {
    v: u32,
}

#[derive(Deserialize)]
struct Read {
    profile: Profile,
}

pub(super) fn encode(profile: &Profile) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&Write { v: FORMAT, profile })
}

/// One object of the feed as the replica reports it; `None` for an object that is not a
/// profile at all (its id is not a profile id), which is neither shown nor touched.
pub(super) fn read(
    scope: ReplicaScope,
    collection_id: &str,
    key: &CollectionKey,
    object: &v2::StoredObject,
) -> Result<Option<RemoteProfile>, ReplicaError> {
    let Ok(id) = ProfileId::from_str(&object.object_id) else {
        return Ok(None);
    };
    let body = if object.deleted {
        RemoteBody::Deleted
    } else if object.kind != KIND {
        RemoteBody::Unrecognized
    } else {
        let plain = key
            .open_object(
                collection_id,
                &object.object_id,
                &object.kind,
                &object.ciphertext,
            )
            .map_err(|error| ReplicaError::Other(SharedCause::new(error)))?;
        body_of(id, &plain)
    };
    Ok(Some(RemoteProfile {
        scope,
        id,
        version: ReplicaVersion::new(object.version),
        body,
    }))
}

fn body_of(id: ProfileId, plaintext: &[u8]) -> RemoteBody {
    match serde_json::from_slice::<Head>(plaintext) {
        Ok(Head { v: FORMAT }) => {}
        _ => return RemoteBody::Unrecognized,
    }
    match serde_json::from_slice::<Read>(plaintext) {
        Ok(Read { profile }) if profile.id == id => RemoteBody::Present(Box::new(profile)),
        _ => RemoteBody::Unrecognized,
    }
}

/// The position in the feed of every collection read: JSON, so it stays opaque to the
/// reconciliation and can grow to several collections.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Positions(std::collections::BTreeMap<String, u64>);

impl Positions {
    pub(super) fn parse(cursor: Option<&ReplicaCursor>) -> Self {
        cursor
            .and_then(|cursor| serde_json::from_str(cursor.as_str()).ok())
            .unwrap_or_default()
    }

    pub(super) fn get(&self, collection_id: &str) -> u64 {
        self.0.get(collection_id).copied().unwrap_or(0)
    }

    pub(super) fn set(&mut self, collection_id: &str, position: u64) {
        self.0.insert(collection_id.to_owned(), position);
    }

    pub(super) fn cursor(&self) -> ReplicaCursor {
        ReplicaCursor::new(serde_json::to_string(self).unwrap_or_else(|_| "{}".to_owned()))
    }
}

#[cfg(test)]
mod tests;
