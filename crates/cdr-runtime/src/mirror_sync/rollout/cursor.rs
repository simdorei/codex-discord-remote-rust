use cdr_store::mirror::MirrorOffset;
use rusqlite::{Connection, OptionalExtension};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

pub(super) fn read(db: &Connection, thread: &str) -> rusqlite::Result<Option<MirrorOffset>> {
    db.query_row(
        "SELECT rollout_path,cursor,updated_at FROM codex_session_mirror_offsets WHERE codex_thread_id=?",
        [thread],
        |row| Ok(MirrorOffset { rollout_path: row.get(0)?, cursor: row.get(1)?, updated_at: row.get(2)? }),
    ).optional()
}

pub(super) fn validate(
    file: &mut File,
    path: &Path,
    offset: Option<&MirrorOffset>,
) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "rollout is not a regular file",
        ));
    }
    let Some(offset) = offset else {
        return Ok(());
    };
    let invalid = |reason: &str| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "cursor={} is invalid ({reason}); saved position retained; explicit recovery required; sync does not reset or skip history",
                offset.cursor
            ),
        )
    };
    if offset.rollout_path != path.to_string_lossy() {
        return Err(invalid("stored rollout path differs"));
    }
    let position = u64::try_from(offset.cursor).map_err(|_| invalid("negative position"))?;
    if position > metadata.len() {
        return Err(invalid("past end of file"));
    }
    if position > 0 && position < metadata.len() {
        file.seek(SeekFrom::Start(position - 1))?;
        let mut previous = [0];
        file.read_exact(&mut previous)?;
        if previous != *b"\n" {
            return Err(invalid("not at a record boundary"));
        }
    }
    Ok(())
}
