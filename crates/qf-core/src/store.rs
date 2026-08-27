use crate::Store;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

const DIRECTORY_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;

/// A save failure, including whether the destination was already replaced.
///
/// Before the commit point, callers can safely retain or restore their old
/// in-memory state. After the commit point, the new JSON is visible at the
/// destination even though syncing its directory failed.
#[derive(Debug)]
pub enum SaveError {
    BeforeCommit(io::Error),
    AfterCommit(io::Error),
}

impl SaveError {
    /// Whether the atomic rename that installs the new JSON has completed.
    pub fn is_committed(&self) -> bool {
        matches!(self, Self::AfterCommit(_))
    }

    pub fn kind(&self) -> io::ErrorKind {
        self.error().kind()
    }

    fn error(&self) -> &io::Error {
        match self {
            Self::BeforeCommit(error) | Self::AfterCommit(error) => error,
        }
    }
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error().fmt(f)
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.error())
    }
}

/// `$XDG_DATA_HOME/queue-focus/tasks.json` (defaults to `~/.local/share`).
pub fn data_path() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("queue-focus").join("tasks.json")
}

/// Load the task store, repairing permissions left by pre-hardening builds.
///
/// The app directory and store file contain potentially private task titles,
/// so they are restricted to the current user. `O_NOFOLLOW` prevents an
/// accidental final-component symlink from having its target chmodded or read.
pub fn load(path: &Path) -> io::Result<Store> {
    let directory = store_dir(path);
    if directory != Path::new(".") {
        let metadata = match fs::metadata(directory) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Store::new()),
            Err(error) => return Err(error),
        };
        if !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "task store parent is not a directory",
            ));
        }
        fs::set_permissions(directory, fs::Permissions::from_mode(DIRECTORY_MODE))?;
    }

    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Store::new()),
        Err(error) => return Err(error),
    };

    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "task store is not a regular file",
        ));
    }
    file.set_permissions(fs::Permissions::from_mode(FILE_MODE))?;

    serde_json::from_reader(file).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Atomically save the store and sync both the file and its directory.
pub fn save(path: &Path, store: &Store) -> Result<(), SaveError> {
    let json = serde_json::to_vec_pretty(store).map_err(|error| {
        SaveError::BeforeCommit(io::Error::new(io::ErrorKind::InvalidData, error))
    })?;
    save_bytes_with_sync(path, &json, sync_directory)
}

fn save_bytes_with_sync(
    path: &Path,
    contents: &[u8],
    sync: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(), SaveError> {
    let file_name = path.file_name().ok_or_else(|| {
        SaveError::BeforeCommit(io::Error::new(
            io::ErrorKind::InvalidInput,
            "task store path has no file name",
        ))
    })?;
    let directory = prepare_store_dir(path).map_err(SaveError::BeforeCommit)?;
    let (temp_path, mut file) =
        create_private_temp(directory, file_name).map_err(SaveError::BeforeCommit)?;

    let commit = (|| -> io::Result<()> {
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp_path, path)
    })();

    if let Err(error) = commit {
        let _ = fs::remove_file(&temp_path);
        return Err(SaveError::BeforeCommit(error));
    }

    sync(directory).map_err(SaveError::AfterCommit)
}

fn prepare_store_dir(path: &Path) -> io::Result<&Path> {
    let directory = store_dir(path);
    if directory == Path::new(".") {
        return Ok(directory);
    }

    fs::create_dir_all(directory)?;
    if !fs::metadata(directory)?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "task store parent is not a directory",
        ));
    }
    fs::set_permissions(directory, fs::Permissions::from_mode(DIRECTORY_MODE))?;
    Ok(directory)
}

fn store_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|directory| !directory.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn create_private_temp(directory: &Path, file_name: &OsStr) -> io::Result<(PathBuf, File)> {
    for _ in 0..16 {
        let nonce = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let mut temp_name = OsString::from(".");
        temp_name.push(file_name);
        temp_name.push(format!(".tmp-{}-{nonce}", std::process::id()));
        let temp_path = directory.join(temp_name);

        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(FILE_MODE)
            .open(&temp_path)
        {
            Ok(file) => {
                if let Err(error) = file.set_permissions(fs::Permissions::from_mode(FILE_MODE)) {
                    drop(file);
                    let _ = fs::remove_file(&temp_path);
                    return Err(error);
                }
                return Ok((temp_path, file));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create a unique task temporary file",
    ))
}

fn sync_directory(directory: &Path) -> io::Result<()> {
    File::open(directory)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Bucket;
    use std::os::unix::fs::symlink;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("qf-store-{name}-{}-{nonce}", std::process::id()))
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn save_load_roundtrip() {
        let root = temp_dir("roundtrip");
        let path = root.join("queue-focus/tasks.json");
        assert!(load(&path).unwrap().is_empty());

        let mut store = Store::new();
        store.add("x", Bucket::Now, None, false);
        save(&path, &store).unwrap();

        let loaded = load(&path).unwrap();
        assert_eq!(loaded.tasks, store.tasks);
        assert_eq!(loaded.next_id, store.next_id);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn save_uses_private_permissions_on_create_and_replace() {
        let root = temp_dir("save-permissions");
        let directory = root.join("queue-focus");
        let path = directory.join("tasks.json");
        let mut store = Store::new();
        store.add("private task", Bucket::Now, None, false);

        save(&path, &store).unwrap();
        assert_eq!(mode(&directory), DIRECTORY_MODE);
        assert_eq!(mode(&path), FILE_MODE);

        fs::set_permissions(&directory, fs::Permissions::from_mode(0o775)).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o664)).unwrap();
        save(&path, &store).unwrap();
        assert_eq!(mode(&directory), DIRECTORY_MODE);
        assert_eq!(mode(&path), FILE_MODE);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn load_repairs_existing_permissions_before_parsing() {
        let root = temp_dir("load-permissions");
        let directory = root.join("queue-focus");
        let path = directory.join("tasks.json");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&path, b"{ not valid json").unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o775)).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o664)).unwrap();

        assert_eq!(load(&path).unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(mode(&directory), DIRECTORY_MODE);
        assert_eq!(mode(&path), FILE_MODE);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn load_rejects_a_symlink_without_touching_its_target() {
        let root = temp_dir("symlink");
        let directory = root.join("queue-focus");
        let path = directory.join("tasks.json");
        let target = root.join("unrelated.json");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&target, serde_json::to_vec(&Store::new()).unwrap()).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o664)).unwrap();
        symlink(&target, &path).unwrap();

        assert!(load(&path).is_err());
        assert_eq!(mode(&target), 0o664);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_directory_sync_failure_is_reported_after_commit() {
        let root = temp_dir("sync-failure");
        let path = root.join("queue-focus/tasks.json");

        let error = save_bytes_with_sync(&path, b"committed contents", |_| {
            Err(io::Error::other("injected directory sync failure"))
        })
        .unwrap_err();

        assert!(error.is_committed());
        assert_eq!(fs::read(&path).unwrap(), b"committed contents");
        let entries: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, [OsString::from("tasks.json")]);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_failed_rename_removes_the_temporary_file() {
        let root = temp_dir("rename-failure");
        let path = root.join("queue-focus/tasks.json");
        fs::create_dir_all(&path).unwrap();

        let error = save_bytes_with_sync(&path, b"not committed", |_| Ok(())).unwrap_err();

        assert!(!error.is_committed());
        let entries: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, [OsString::from("tasks.json")]);

        fs::remove_dir_all(root).unwrap();
    }
}
