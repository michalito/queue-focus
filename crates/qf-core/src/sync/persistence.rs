use crate::SaveError;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::{
        fd::AsRawFd,
        unix::fs::{OpenOptionsExt, PermissionsExt},
    },
    path::Path,
};

/// A host or setup utility holds this for its entire lifetime. Never unlink
/// the lock file: doing so would allow two different inodes to be locked.
#[derive(Debug)]
pub struct DataLock {
    _file: File,
}
impl DataLock {
    pub fn acquire(directory: &Path) -> io::Result<Self> {
        fs::create_dir_all(directory)?;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(directory.join(".queue-focus.lock"))?;
        if !file.metadata()?.is_file() {
            return Err(invalid("data lock is not a regular file"));
        }
        // SAFETY: the descriptor is owned by `file` for this call and the
        // returned guard's lifetime; flock is released when it is closed.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Queue Focus is using this data directory. Quit the app before setup or recovery.",
            ));
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        Ok(Self { _file: file })
    }
}

pub fn read_private<T: DeserializeOwned>(path: &Path, limit: usize) -> io::Result<T> {
    let bytes = read_private_bytes(path, limit)?;
    serde_json::from_slice(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn read_private_bytes(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(invalid("invalid or oversized sync file"));
    }
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit || bytes.iter().copied().find(|b| !b.is_ascii_whitespace()) != Some(b'{')
    {
        return Err(invalid("sync file must be a bounded JSON object"));
    }
    Ok(bytes)
}

pub fn save_private(path: &Path, value: &impl Serialize, limit: usize) -> Result<(), SaveError> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| SaveError::BeforeCommit(io::Error::new(io::ErrorKind::InvalidData, e)))?;
    save_private_bytes(path, &bytes, limit)
}

pub fn save_private_bytes(path: &Path, bytes: &[u8], limit: usize) -> Result<(), SaveError> {
    if bytes.len() > limit {
        return Err(SaveError::BeforeCommit(invalid(
            "sync storage limit reached; export or dismiss recovery items before retrying",
        )));
    }
    crate::store::save_bytes_with_sync(path, bytes, |directory| File::open(directory)?.sync_all())
}

pub fn sync_parent(path: &Path) -> io::Result<()> {
    File::open(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new(".")),
    )?
    .sync_all()
}
pub(super) fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
