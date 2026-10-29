//! Offline partial-restore fault fixture, never an authenticated writer.
use std::path::Path;

pub(crate) struct RootStateSnapshot {
    catalog: (i64, Vec<u8>),
    cell: (i64, Option<i64>, Vec<u8>, Vec<u8>),
    header_fields: [i64; 8],
    header_digest: Vec<u8>,
    header_length: i64,
}
impl RootStateSnapshot {
    pub(crate) fn capture(root: &Path) -> Self {
        let db = rusqlite::Connection::open(root.join("cloud-runs.sqlite3")).unwrap();
        let catalog = db
            .query_row(
                "SELECT revision,body FROM account_catalog WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let cell = db
            .query_row(
                "SELECT revision,writer,digest,body FROM current_heads",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        let header_fields = db
            .query_row(
                "SELECT writer,revision,records,events,control,retained,deleted,charge FROM chats",
                [],
                |row| {
                    Ok([
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ])
                },
            )
            .unwrap();
        let (header_digest, header_length) = db
            .query_row("SELECT header_digest,length(body) FROM chats", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        Self {
            catalog,
            cell,
            header_fields,
            header_digest,
            header_length,
        }
    }
    pub(crate) fn restore_root_rows(&self, root: &Path) {
        let db = rusqlite::Connection::open(root.join("cloud-runs.sqlite3")).unwrap();
        db.execute(
            "UPDATE account_catalog SET revision=?1,body=?2",
            rusqlite::params![self.catalog.0, self.catalog.1],
        )
        .unwrap();
        db.execute(
            "UPDATE current_heads SET revision=?1,writer=?2,digest=?3,body=?4",
            rusqlite::params![self.cell.0, self.cell.1, self.cell.2, self.cell.3],
        )
        .unwrap();
    }
    pub(crate) fn restore_header_index_without_its_ciphertext(&self, root: &Path) {
        let db = rusqlite::Connection::open(root.join("cloud-runs.sqlite3")).unwrap();
        let length: i64 = db
            .query_row("SELECT length(body) FROM chats", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            length, self.header_length,
            "exercise the actual ciphertext hash, not just a length mismatch"
        );
        db.execute("UPDATE chats SET writer=?1,revision=?2,records=?3,events=?4,control=?5,retained=?6,deleted=?7,charge=?8,header_digest=?9",rusqlite::params![self.header_fields[0],self.header_fields[1],self.header_fields[2],self.header_fields[3],self.header_fields[4],self.header_fields[5],self.header_fields[6],self.header_fields[7],self.header_digest]).unwrap();
    }
}
