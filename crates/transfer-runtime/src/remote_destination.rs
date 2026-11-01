//! Destination-side publication of one remote file: name reservation, staged
//! writer, collision handling and atomic publication. Shared by uploads and relays.
use crate::{
    reservations,
    scheduler::{FileOutcome, Work},
};
use nocterm_session::{
    FsError,
    fs::{RemoteUpload, UploadMode, path},
};
use nocterm_transfers::{CollisionPolicy, TransferCause};

pub(crate) async fn open_writer(
    work: &Work,
    original: &str,
) -> Result<
    Option<(
        String,
        reservations::Reservation,
        u32,
        Box<dyn RemoteUpload>,
    )>,
    TransferCause,
> {
    let mode = mode(work);
    let mut number = 0;
    loop {
        let Some((destination, reservation, used)) =
            reservations::remote(work, original, number).await?
        else {
            return Ok(None);
        };
        match work
            .remote(work.batch().request().fs().upload(&destination, mode))
            .await
        {
            Ok(writer) => return Ok(Some((destination, reservation, used, writer))),
            Err(TransferCause::Remote(FsError::AlreadyExists { .. })) => {
                drop(reservation);
                match work.batch().request().collisions() {
                    CollisionPolicy::Skip => return Ok(None),
                    CollisionPolicy::Rename => number = reservations::next_number(work, used)?,
                    CollisionPolicy::Replace => {
                        return Err(TransferCause::Remote(FsError::AlreadyExists {
                            path: destination,
                        }));
                    }
                }
            }
            Err(cause) => return Err(cause),
        }
    }
}
pub(crate) async fn publish(
    work: &Work,
    original: &str,
    mut destination: String,
    mut number: u32,
    reservation: &mut reservations::Reservation,
    writer: &mut dyn RemoteUpload,
) -> Result<FileOutcome, TransferCause> {
    loop {
        work.check()?;
        // Publication may commit while cancellation arrives. Await its answer
        // so a completed rename is never mistaken for an abandoned stage.
        match writer
            .publish_as(path::file_name(&destination).to_owned())
            .await
            .map_err(TransferCause::from)
        {
            Ok(_) => return Ok(FileOutcome::Completed),
            Err(TransferCause::Remote(FsError::AlreadyExists { .. }))
                if work.batch().request().collisions() == CollisionPolicy::Skip =>
            {
                return Ok(FileOutcome::Skipped(None));
            }
            Err(TransferCause::Remote(FsError::AlreadyExists { .. }))
                if work.batch().request().collisions() == CollisionPolicy::Rename =>
            {
                let Some((next_name, next_reservation, used)) =
                    reservations::remote(work, original, reservations::next_number(work, number)?)
                        .await?
                else {
                    return Ok(FileOutcome::Skipped(None));
                };
                *reservation = next_reservation;
                destination = next_name;
                number = used;
            }
            Err(cause) => return Err(cause),
        }
    }
}
fn mode(work: &Work) -> UploadMode {
    if work.batch().request().collisions() == CollisionPolicy::Replace {
        UploadMode::Replace
    } else {
        UploadMode::Create
    }
}
