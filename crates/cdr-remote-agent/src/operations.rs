use cdr_remote_protocol::message::WriteFileOutput;
use cdr_remote_protocol::output::CoreOutput;

use crate::checkpoints::{CheckpointError, mutate};
use crate::files::{ProjectFileAccess, RemoteFileError, hex_digest};

pub fn create_file(
    access: &ProjectFileAccess,
    path: &str,
    content: &str,
    overwrite: bool,
) -> Result<CoreOutput, CheckpointError> {
    let exists = access.file_exists(path)?;
    if exists && !overwrite {
        return Err(RemoteFileError::Conflict {
            path: path.to_owned(),
            reason: "file already exists".into(),
        }
        .into());
    }
    let expected = if exists {
        Some(hex_digest(
            &access.read_bytes(path, crate::files::MAX_FILE_BYTES)?,
        ))
    } else {
        None
    };
    let paths = [path.to_owned()];
    let (written, checkpoint_id) = mutate(access, "create", &paths, |transaction| {
        let written = access.write_file(path, content, expected.as_deref())?;
        transaction.record_write(path, written.sha256.clone());
        Ok(written)
    })?;
    Ok(CoreOutput::FileCreate {
        path: written.path,
        sha256: written.sha256,
        bytes_written: written.bytes_written,
        checkpoint_id,
    })
}

pub fn write_with_checkpoint(
    access: &ProjectFileAccess,
    path: &str,
    content: &str,
    expected_sha256: Option<&str>,
) -> Result<WriteFileOutput, CheckpointError> {
    let paths = [path.to_owned()];
    let (written, _) = mutate(access, "write file", &paths, |transaction| {
        let written = access.write_file(path, content, expected_sha256)?;
        transaction.record_write(path, written.sha256.clone());
        Ok(written)
    })?;
    Ok(written)
}
