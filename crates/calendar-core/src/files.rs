//! Atomic file writes (conventions section 5): the Rust form of Swift's `.atomic` write option.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A random suffix for the temporary name. `getrandom` is the source; if it fails the process id and a counter
/// still keep concurrent writers apart.
fn random_suffix() -> String {
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed) ^ (u64::from(std::process::id()) << 32);
        bytes = n.to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Writes `bytes` to `<name>.tmp-<random>` in the directory of `path`, flushes it to disk, and renames it over
/// `path`. The old content stays intact when any step fails, and the temporary file is removed.
///
/// A missing parent directory, or a path without a file name, is an error.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let mut tmp_name = name.to_os_string();
    tmp_name.push(format!(".tmp-{}", random_suffix()));
    let tmp = path.with_file_name(tmp_name);

    let written = (|| -> io::Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}
