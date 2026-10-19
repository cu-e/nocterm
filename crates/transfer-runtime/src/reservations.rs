use crate::scheduler::{DownloadJob, Work};
use nocterm_session::{fs::path, local_fs::DirectoryIdentity};
use nocterm_transfers::{CollisionDecision, LimitKind, TransferCause, collision_decision, renamed};
use parking_lot::Mutex;
use std::{collections::HashSet, ffi::OsString, sync::Arc};

#[derive(Default)]
pub(crate) struct ReservationBook {
    remote: Mutex<HashSet<String>>,
    local: Mutex<HashSet<(DirectoryIdentity, OsString)>>,
}
enum Key {
    Remote(String),
    Local(DirectoryIdentity, OsString),
}
pub(crate) struct Reservation {
    book: Arc<ReservationBook>,
    key: Key,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        match &self.key {
            Key::Remote(path) => {
                self.book.remote.lock().remove(path);
            }
            Key::Local(identity, name) => {
                self.book
                    .local
                    .lock()
                    .remove(&(identity.clone(), name.clone()));
            }
        }
    }
}
impl ReservationBook {
    fn remote_reserved(&self, path: &str) -> bool {
        self.remote.lock().contains(path)
    }
    fn local_reserved(&self, identity: &DirectoryIdentity, name: &std::ffi::OsStr) -> bool {
        self.local
            .lock()
            .contains(&(identity.clone(), name.to_owned()))
    }
    fn remote_claim(self: &Arc<Self>, path: String) -> Option<Reservation> {
        self.remote
            .lock()
            .insert(path.clone())
            .then(|| Reservation {
                book: self.clone(),
                key: Key::Remote(path),
            })
    }
    fn local_claim(
        self: &Arc<Self>,
        identity: DirectoryIdentity,
        name: OsString,
    ) -> Option<Reservation> {
        self.local
            .lock()
            .insert((identity.clone(), name.clone()))
            .then(|| Reservation {
                book: self.clone(),
                key: Key::Local(identity, name),
            })
    }
}
pub(crate) async fn remote(
    work: &Work,
    original: &str,
    start: u32,
) -> Result<Option<(String, Reservation, u32)>, TransferCause> {
    let book = work.batch().reservations();
    let mut number = start;
    loop {
        work.check()?;
        let candidate = if number == 0 {
            original.to_owned()
        } else {
            renamed(original, number)
        };
        let exists = work
            .remote(work.batch().request().fs().stat(&candidate))
            .await?
            .is_some();
        match collision_decision(
            work.batch().request().collisions(),
            exists,
            book.remote_reserved(&candidate),
        ) {
            CollisionDecision::Skip => return Ok(None),
            CollisionDecision::Use => {
                if let Some(reservation) = book.remote_claim(candidate.clone()) {
                    return Ok(Some((candidate, reservation, number)));
                }
            }
            CollisionDecision::Rename => number = next_number(work, number)?,
            CollisionDecision::Wait => {
                tokio::select! {()=work.cancelled()=>return Err(TransferCause::Cancelled),()=tokio::time::sleep(std::time::Duration::from_millis(10))=>{}}
            }
        }
    }
}
pub(crate) async fn local(
    work: &Work,
    job: &DownloadJob,
    start: u32,
) -> Result<Option<(OsString, Reservation, u32)>, TransferCause> {
    let identity = job.directory().identity();
    let book = work.batch().reservations();
    let mut number = start;
    loop {
        work.check()?;
        let original = job.name().to_str().ok_or(TransferCause::InvalidName)?;
        let name: OsString = if number == 0 {
            original.to_owned()
        } else {
            path::file_name(&renamed(original, number)).to_owned()
        }
        .into();
        let directory = job.directory().clone();
        let probe = name.clone();
        let exists = work
            .state()
            .blocking()
            .call(work.cancel(), move || directory.metadata(&probe))
            .await?
            .is_some();
        match collision_decision(
            work.batch().request().collisions(),
            exists,
            book.local_reserved(&identity, &name),
        ) {
            CollisionDecision::Skip => return Ok(None),
            CollisionDecision::Use => {
                if let Some(reservation) = book.local_claim(identity.clone(), name.clone()) {
                    return Ok(Some((name, reservation, number)));
                }
            }
            CollisionDecision::Rename => number = next_number(work, number)?,
            CollisionDecision::Wait => {
                tokio::select! {()=work.cancelled()=>return Err(TransferCause::Cancelled),()=tokio::time::sleep(std::time::Duration::from_millis(10))=>{}}
            }
        }
    }
}
fn next_number(work: &Work, number: u32) -> Result<u32, TransferCause> {
    let next = number
        .checked_add(1)
        .ok_or(TransferCause::Limit(LimitKind::RenameAttempts))?;
    if next > work.state().limits().discovery().rename_attempts() {
        return Err(TransferCause::Limit(LimitKind::RenameAttempts));
    }
    Ok(next)
}
