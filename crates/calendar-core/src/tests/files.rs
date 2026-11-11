use std::fs;
use std::path::PathBuf;

use crate::files::write_atomic;

/// A directory under the system temp dir, removed at the end of the test.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let mut path = std::env::temp_dir();
        path.push(format!("calendar-core-files-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn names(dir: &TempDir) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(&dir.0)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn write_atomic_creates_and_replaces_content() {
    let dir = TempDir::new("replace");
    let target = dir.0.join("connections.json");
    write_atomic(&target, b"first").unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"first");
    write_atomic(&target, b"second, longer").unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"second, longer");
    write_atomic(&target, b"").unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"");
}

#[test]
fn write_atomic_leaves_no_temporary_file() {
    let dir = TempDir::new("tmp");
    let target = dir.0.join("state.json");
    for i in 0..5 {
        write_atomic(&target, format!("v{i}").as_bytes()).unwrap();
    }
    assert_eq!(names(&dir), vec!["state.json".to_string()]);
}

#[test]
fn write_atomic_into_a_missing_directory_is_an_error() {
    let dir = TempDir::new("missing");
    let target = dir.0.join("no-such-dir").join("file.json");
    assert!(write_atomic(&target, b"x").is_err());
    assert!(names(&dir).is_empty());
}

#[test]
fn write_atomic_without_a_file_name_is_an_error() {
    let dir = TempDir::new("noname");
    assert!(write_atomic(&dir.0.join(".."), b"x").is_err());
    assert!(write_atomic(std::path::Path::new(""), b"x").is_err());
}

#[test]
fn write_atomic_failure_keeps_the_old_content() {
    let dir = TempDir::new("keep");
    let target = dir.0.join("keep.json");
    write_atomic(&target, b"old").unwrap();
    // A directory in the way of the rename: the write fails, the temporary file is removed.
    let blocked = dir.0.join("blocked");
    fs::create_dir_all(blocked.join("child")).unwrap();
    assert!(write_atomic(&blocked, b"new").is_err());
    assert_eq!(fs::read(&target).unwrap(), b"old");
    assert_eq!(
        names(&dir),
        vec!["blocked".to_string(), "keep.json".to_string()]
    );
}
