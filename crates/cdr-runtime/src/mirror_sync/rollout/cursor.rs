use cdr_store::mirror::MirrorOffset;
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    fs::File,
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom},
    path::Path,
};

const MAX_RECOVERY_RECORD: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Recovery {
    pub(super) from: i64,
    pub(super) to: i64,
}

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
) -> io::Result<Option<Recovery>> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "rollout is not a regular file",
        ));
    }
    let Some(offset) = offset else {
        return Ok(None);
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
    if position > 0 {
        file.seek(SeekFrom::Start(position - 1))?;
        let mut previous = [0];
        file.read_exact(&mut previous)?;
        if previous != *b"\n" {
            let target = recovery_target(file, position).map_err(|error| {
                invalid(&format!(
                    "not at a record boundary; automatic rewind refused: {error}"
                ))
            })?;
            return Ok(Some(Recovery {
                from: offset.cursor,
                to: i64::try_from(target)
                    .map_err(|_| invalid("recovery position exceeds database range"))?,
            }));
        }
    }
    Ok(None)
}

pub(super) fn persist(
    connection: &Connection,
    thread: &str,
    path: &Path,
    recovery: Recovery,
    now: f64,
) -> rusqlite::Result<bool> {
    Ok(connection.execute(
        "UPDATE codex_session_mirror_offsets SET cursor=?1,updated_at=?2 \
         WHERE codex_thread_id=?3 AND rollout_path=?4 AND cursor=?5",
        params![
            recovery.to,
            now,
            thread,
            path.to_string_lossy().as_ref(),
            recovery.from
        ],
    )? == 1)
}

fn recovery_target(file: &mut File, position: u64) -> io::Result<u64> {
    const WINDOW: usize = 8192;
    let mut end = position - 1;
    let mut buffer = [0_u8; WINDOW];
    let start = loop {
        if end == 0 {
            break 0;
        }
        let begin = end.saturating_sub(WINDOW as u64);
        let length = usize::try_from(end - begin).map_err(io::Error::other)?;
        file.seek(SeekFrom::Start(begin))?;
        file.read_exact(&mut buffer[..length])?;
        if let Some(index) = buffer[..length].iter().rposition(|byte| *byte == b'\n') {
            break begin + u64::try_from(index).map_err(io::Error::other)? + 1;
        }
        end = begin;
    };
    file.seek(SeekFrom::Start(start))?;
    let mut record = Vec::new();
    BufReader::new((&mut *file).take(MAX_RECOVERY_RECORD + 1)).read_until(b'\n', &mut record)?;
    if record.len() as u64 > MAX_RECOVERY_RECORD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "containing record exceeds the 1 MiB recovery limit",
        ));
    }
    if record.last() != Some(&b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "containing record is incomplete",
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(&record[..record.len() - 1])
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if !value.is_object() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "containing record is not a JSON object",
        ));
    }
    Ok(start)
}
