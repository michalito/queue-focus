use crate::{Settings, Store};
use serde::de::DeserializeOwned;
use serde::Serialize;
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

/// `$XDG_DATA_HOME/queue-focus` (defaults to `~/.local/share/queue-focus`).
pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("queue-focus")
}

/// `$XDG_DATA_HOME/queue-focus/tasks.json`.
pub fn data_path() -> PathBuf {
    data_dir().join("tasks.json")
}

/// `$XDG_DATA_HOME/queue-focus/settings.json`.
pub fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

/// Load the task store, repairing permissions left by pre-hardening builds.
///
/// An absent file or directory is an empty queue, not an error.
pub fn load(path: &Path) -> io::Result<Store> {
    Ok(read_json(path)?.unwrap_or_else(Store::new))
}

/// Load the settings. An absent file means the defaults; a malformed one is
/// an error, so a caller can say so rather than quietly resetting the file.
pub fn load_settings(path: &Path) -> io::Result<Settings> {
    let mut settings: Settings = read_json(path)?.unwrap_or_default();
    settings.sanitize();
    Ok(settings)
}

/// Read one of our JSON files, or `None` when it does not exist yet.
///
/// The app directory and its files contain potentially private task titles,
/// so they are restricted to the current user. `O_NOFOLLOW` prevents an
/// accidental final-component symlink from having its target chmodded or read.
///
/// Both our files are JSON objects. Serde would happily read a struct from an
/// array instead, filling the fields in declaration order, so a file whose
/// shape is wrong would come back as a plausible-looking value rather than as
/// the error it is.
fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    let directory = store_dir(path);
    if directory != Path::new(".") {
        let metadata = match fs::metadata(directory) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if !metadata.is_dir() {
            return Err(bad_data(format!(
                "{} is not a directory",
                directory.display()
            )));
        }
        fs::set_permissions(directory, fs::Permissions::from_mode(DIRECTORY_MODE))?;
    }

    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };

    if !file.metadata()?.is_file() {
        return Err(bad_data(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    file.set_permissions(fs::Permissions::from_mode(FILE_MODE))?;

    let json: serde_json::Value = serde_json::from_reader(file)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if !json.is_object() {
        return Err(bad_data(format!(
            "{} does not hold a JSON object",
            path.display()
        )));
    }
    serde_json::from_value(json)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn bad_data(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Atomically save the store and sync both the file and its directory.
pub fn save(path: &Path, store: &Store) -> Result<(), SaveError> {
    save_json(path, store)
}

/// Save the settings the same way, so a half-written file never replaces a
/// good one.
pub fn save_settings(path: &Path, settings: &Settings) -> Result<(), SaveError> {
    save_json(path, settings)
}

fn save_json<T: Serialize>(path: &Path, value: &T) -> Result<(), SaveError> {
    let json = serde_json::to_vec_pretty(value).map_err(|error| {
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
            format!("{} has no file name", path.display()),
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
        return Err(bad_data(format!(
            "{} is not a directory",
            directory.display()
        )));
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
    fn settings_round_trip_and_start_from_the_defaults() {
        let root = temp_dir("settings");
        let path = root.join("queue-focus/settings.json");
        assert_eq!(load_settings(&path).unwrap(), Settings::default());

        let settings = Settings {
            interval_min: 25,
            theme: crate::Theme::Dark,
            ..Settings::default()
        };
        save_settings(&path, &settings).unwrap();
        // Asserted before the load, which would repair the modes itself and
        // hide anything the save got wrong.
        assert_eq!(mode(&path), FILE_MODE);
        assert_eq!(mode(path.parent().unwrap()), DIRECTORY_MODE);

        assert_eq!(load_settings(&path).unwrap(), settings);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_hand_edited_settings_file_is_pulled_back_into_range_on_load() {
        let root = temp_dir("settings-clamp");
        let directory = root.join("queue-focus");
        let path = directory.join("settings.json");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&path, br#"{"interval_min": 4000}"#).unwrap();

        assert_eq!(
            load_settings(&path).unwrap().interval_min,
            crate::INTERVAL_MAX
        );

        fs::remove_dir_all(root).unwrap();
    }

    /// Settings live beside the tasks, so a malformed one is reported rather
    /// than silently replaced — the same rule the task file follows.
    #[test]
    fn malformed_settings_are_reported_and_left_alone() {
        let root = temp_dir("settings-malformed");
        let directory = root.join("queue-focus");
        let path = directory.join("settings.json");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&path, b"{ not settings").unwrap();

        assert_eq!(
            load_settings(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(fs::read(&path).unwrap(), b"{ not settings");

        fs::remove_dir_all(root).unwrap();
    }

    /// Serde reads a struct from a JSON array too, taking the fields in
    /// declaration order. A file truncated to `[]`, or a hand-edit that turned
    /// the object into a list, must be reported rather than quietly accepted
    /// as a plausible set of values.
    #[test]
    fn a_file_that_is_not_a_json_object_is_refused() {
        let root = temp_dir("wrong-shape");
        let directory = root.join("queue-focus");
        fs::create_dir_all(&directory).unwrap();

        for contents in [
            &b"[]"[..],
            b"[7, false]",
            br#"[3,false,"strong","orange",true,true,"22:00","06:00","dark",false,"side"]"#,
            b"\"settings\"",
            b"null",
            b"12",
        ] {
            let path = directory.join("settings.json");
            fs::write(&path, contents).unwrap();
            let error = load_settings(&path).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(
                error.to_string().contains("does not hold a JSON object"),
                "{}",
                error
            );
        }

        let path = directory.join("tasks.json");
        fs::write(&path, b"[1, []]").unwrap();
        assert!(load(&path).is_err(), "the task file is held to it too");

        fs::remove_dir_all(root).unwrap();
    }

    /// The read and save paths are shared by both files, so their complaints
    /// have to name the file they were actually given.
    #[test]
    fn a_problem_with_a_file_names_that_file() {
        let root = temp_dir("named-errors");
        let directory = root.join("queue-focus");
        let path = directory.join("settings.json");
        // A directory where the file belongs.
        fs::create_dir_all(&path).unwrap();

        let error = load_settings(&path).unwrap_err().to_string();
        assert!(error.contains("settings.json"), "{error}");
        assert!(!error.contains("task store"), "{error}");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_data_files_sit_together_in_one_directory() {
        assert_eq!(data_path().parent(), Some(data_dir().as_path()));
        assert_eq!(settings_path().parent(), Some(data_dir().as_path()));
        assert_eq!(data_path().file_name().unwrap(), "tasks.json");
        assert_eq!(settings_path().file_name().unwrap(), "settings.json");
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
