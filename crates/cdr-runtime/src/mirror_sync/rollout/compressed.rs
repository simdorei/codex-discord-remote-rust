use same_file::Handle;
use std::{
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};
use tempfile::NamedTempFile;

pub(super) struct Staged {
    pub(super) file: NamedTempFile,
    source: File,
    source_path: PathBuf,
    identity: Handle,
    length: u64,
    modified: SystemTime,
}

pub(super) fn stage(path: &Path, thread: &str) -> io::Result<Option<Staged>> {
    let mut source_name = path.as_os_str().to_os_string();
    source_name.push(".zst");
    let source_path = PathBuf::from(source_name);
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ: pin the archive against writes and replacement.
        options.share_mode(1);
    }
    let mut source = match options.open(&source_path) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    source.try_lock_shared().map_err(io::Error::other)?;
    let metadata = source.metadata()?;
    if !metadata.is_file() {
        return Err(invalid("compressed rollout is not a regular file"));
    }
    let identity = Handle::from_file(source.try_clone()?)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut file = tempfile::Builder::new()
        .prefix(".cdr-rollout-")
        .tempfile_in(parent)?;
    {
        let mut decoder = zstd::stream::read::Decoder::new(&mut source)?;
        let mut writer = BufWriter::new(file.as_file_mut());
        io::copy(&mut decoder, &mut writer)?;
        writer.flush()?;
    }
    file.as_file().sync_all()?;
    validate_identity(file.as_file_mut(), thread)?;
    Ok(Some(Staged {
        file,
        source,
        source_path,
        identity,
        length: metadata.len(),
        modified: metadata.modified()?,
    }))
}

impl Staged {
    pub(super) fn publish(self, path: &Path) -> io::Result<()> {
        let metadata = self.source.metadata()?;
        if metadata.len() != self.length
            || metadata.modified()? != self.modified
            || Handle::from_path(&self.source_path)? != self.identity
        {
            return Err(invalid(
                "compressed rollout changed during restoration; retry sync after writers stop",
            ));
        }
        // A competing writer's plain file is never overwritten, including on retry.
        self.file
            .persist_noclobber(path)
            .map_err(|error| error.error)?;
        Ok(())
    }
}

fn validate_identity(file: &mut File, thread: &str) -> io::Result<()> {
    const MAX_METADATA: u64 = 1024 * 1024;
    file.rewind()?;
    let mut first = Vec::new();
    BufReader::new((&mut *file).take(MAX_METADATA)).read_until(b'\n', &mut first)?;
    if first.last() != Some(&b'\n') {
        return Err(invalid(
            "compressed rollout has no complete session metadata within 1 MiB",
        ));
    }
    let record: serde_json::Value = serde_json::from_slice(&first)
        .map_err(|error| invalid(&format!("invalid session metadata: {error}")))?;
    if record["type"] != "session_meta" || record["payload"]["id"].as_str() != Some(thread) {
        return Err(invalid(
            "compressed rollout session id does not match the requested thread",
        ));
    }
    file.seek(SeekFrom::End(-1))?;
    let mut last = [0];
    file.read_exact(&mut last)?;
    if last != *b"\n" {
        return Err(invalid("compressed rollout ends with an incomplete record"));
    }
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
#[path = "compressed_tests.rs"]
mod tests;
