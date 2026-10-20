//! The data files, pinned. The GNOME app and the Mac app both read and write
//! them through this crate, so they agree by construction; these golden files
//! catch the format itself drifting, which would break the files every older
//! version wrote. The Mac's engine bindings load the same two files in
//! `macos/QfCore/Tests`.
use qf_core::{load, load_settings, save, save_settings, Engine};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// A directory of its own holding copies of the golden files: reading one
/// tightens its permissions, which must not happen to the checked-in files.
struct Copies(PathBuf);

impl Copies {
    fn new() -> Copies {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "qf-golden-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        for name in ["tasks.json", "settings.json"] {
            fs::copy(fixture(name), dir.join(name)).unwrap();
        }
        Copies(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Copies {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn golden(name: &str) -> String {
    fs::read_to_string(fixture(name)).unwrap()
}

#[test]
fn the_task_file_writes_back_byte_for_byte() {
    let copies = Copies::new();
    let store = load(&copies.path("tasks.json")).unwrap();
    let out = copies.path("written.json");
    save(&out, &store).unwrap();
    assert_eq!(fs::read_to_string(out).unwrap(), golden("tasks.json"));
}

#[test]
fn the_settings_file_writes_back_byte_for_byte() {
    let copies = Copies::new();
    let settings = load_settings(&copies.path("settings.json")).unwrap();
    let out = copies.path("written.json");
    save_settings(&out, &settings).unwrap();
    assert_eq!(fs::read_to_string(out).unwrap(), golden("settings.json"));
}

#[test]
fn the_engine_opens_them_as_they_are_and_leaves_them_so() {
    let copies = Copies::new();
    let (mut engine, warning) = Engine::open(&copies.0, || 1_759_320_000).unwrap();
    assert_eq!(warning, None);
    let state = engine.state_json();
    for title in [
        "Ship v0.5",
        "Write the release notes",
        "Café ☕ with Ana",
        "Renew the domain",
        "Reply to the review",
    ] {
        assert!(state.contains(title), "{title} in {state}");
    }
    assert!(engine.settings_json().contains(r#""quiet_from":"08:30""#));
    assert!(engine.flush().is_none());
    // Opening settles nothing in a well-formed queue, so nothing is rewritten.
    for name in ["tasks.json", "settings.json"] {
        assert_eq!(
            fs::read_to_string(copies.path(name)).unwrap(),
            golden(name),
            "{name}"
        );
    }
}
