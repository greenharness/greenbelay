//! An exclusive advisory file lock, shared by the ledgers and the migration.

use std::fs::File;
use std::io;
use std::path::Path;
use std::time::Duration;

#[cfg(unix)]
const RETRY_PAUSE: Duration = Duration::from_millis(25);

#[cfg(unix)]
fn open(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?;
    crate::journal::private(path, 0o600);
    Ok(file)
}

#[cfg(unix)]
fn flock(file: &File, operation: i32) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    extern "C" {
        fn flock(fd: i32, op: i32) -> i32;
    }
    // SAFETY: `flock` takes a valid open descriptor owned by `file` and an
    // operation flag; it touches no memory.
    if unsafe { flock(file.as_raw_fd(), operation) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Take the lock on `path`, waiting as long as it takes. Held until the
/// returned file is dropped.
#[cfg(unix)]
pub fn lock(path: &Path) -> io::Result<File> {
    const LOCK_EX: i32 = 2;
    let file = open(path)?;
    flock(&file, LOCK_EX)?;
    Ok(file)
}

// holds-until: two holders run at once on Windows. No lock is taken there:
// `File::lock` needs Rust 1.89, above the 1.85 floor. Ends when the floor
// reaches 1.89.
#[cfg(not(unix))]
pub fn lock(path: &Path) -> io::Result<File> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}

/// Take the lock on `path`, retrying without blocking for at most `budget`
/// (a zero budget tries once). `Ok(None)` when another holder still has it.
#[cfg(unix)]
pub fn lock_within(path: &Path, budget: Duration) -> io::Result<Option<File>> {
    const LOCK_EX_NB: i32 = 2 | 4;
    let file = open(path)?;
    let deadline = std::time::Instant::now() + budget;
    loop {
        match flock(&file, LOCK_EX_NB) {
            Ok(()) => return Ok(Some(file)),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
        let now = std::time::Instant::now();
        if now >= deadline {
            return Ok(None);
        }
        std::thread::sleep(RETRY_PAUSE.min(deadline - now));
    }
}

// holds-until: see `lock`; no lock is taken off unix, so there is nothing to wait for.
#[cfg(not(unix))]
pub fn lock_within(path: &Path, _budget: Duration) -> io::Result<Option<File>> {
    lock(path).map(Some)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_held_lock_times_out_and_a_released_one_is_taken() {
        let dir = std::env::temp_dir().join(format!("gp-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x.lock");
        let held = lock_within(&path, Duration::ZERO).unwrap();
        assert!(held.is_some());
        assert!(lock_within(&path, Duration::from_millis(60))
            .unwrap()
            .is_none());
        drop(held);
        assert!(lock_within(&path, Duration::ZERO).unwrap().is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
