//! Best-effort Linux change notifications; callers retain their polling fallback.
use std::ffi::CString;
use std::fs::File;
use std::io::{ErrorKind, Read};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    Unchanged,
    Reconcile,
    Changed,
}

#[derive(Debug)]
pub(crate) struct DirectoryWatch {
    file: File,
    check_at: Instant,
}

impl DirectoryWatch {
    pub(crate) fn new(path: &Path) -> Option<Self> {
        // NONBLOCK keeps the input thread responsive; CLOEXEC avoids leaking
        // watches into applications launched from settings.
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if fd < 0 {
            return None;
        }
        // SAFETY: fd is newly owned and closed by File on every exit path.
        let file = unsafe { File::from_raw_fd(fd) };
        let mask = libc::IN_MODIFY
            | libc::IN_CLOSE_WRITE
            | libc::IN_ATTRIB
            | libc::IN_CREATE
            | libc::IN_DELETE
            | libc::IN_MOVED_FROM
            | libc::IN_MOVED_TO
            | libc::IN_DELETE_SELF
            | libc::IN_MOVE_SELF
            | libc::IN_ONLYDIR;
        // Watching an existing ancestor also handles first-run directories
        // without creating them or retrying failed watches on every key.
        let mut directory = path;
        loop {
            let name = CString::new(directory.as_os_str().as_bytes()).ok()?;
            if unsafe { libc::inotify_add_watch(file.as_raw_fd(), name.as_ptr(), mask) } >= 0 {
                break;
            }
            match std::io::Error::last_os_error().raw_os_error() {
                Some(libc::ENOENT | libc::ENOTDIR) => directory = directory.parent()?,
                _ => return None,
            }
        }
        Some(Self {
            file,
            check_at: Instant::now(),
        })
    }

    #[cfg(test)]
    pub(crate) fn reconcile_next(&mut self) {
        self.check_at = Instant::now() - Duration::from_secs(3);
    }

    pub(crate) fn poll(&mut self) -> Change {
        // A successful read means invalidation, including IN_Q_OVERFLOW and
        // IN_IGNORED. The caller rebuilds both the catalog and its watch, so
        // parsing individual events or draining an unbounded queue is needless.
        let mut buffer = [0u8; 4096];
        match self.file.read(&mut buffer) {
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                // inotify cannot report remote writes or writes through a
                // symlink target outside this directory. Bound stale data and
                // recover watches on replaced ancestors with a periodic reconciliation.
                if self.check_at.elapsed() >= Duration::from_secs(2) {
                    Change::Reconcile
                } else {
                    Change::Unchanged
                }
            }
            _ => Change::Changed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_changes_and_directory_replacement() {
        let root = std::env::temp_dir().join(format!("waypinyin-watch-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut watch = DirectoryWatch::new(&root).expect("inotify watch");
        assert_eq!(watch.poll(), Change::Unchanged);
        let file = root.join("dictionary.tsv");
        std::fs::write(&file, "ni\t你\t0\n").unwrap();
        assert_eq!(watch.poll(), Change::Changed);
        let mut watch = DirectoryWatch::new(&root).unwrap();
        // Equal-length rewrites must invalidate independently of catalog stamps.
        std::fs::write(&file, "ni\t呢\t0\n").unwrap();
        assert_eq!(watch.poll(), Change::Changed);
        let mut watch = DirectoryWatch::new(&root).unwrap();
        std::fs::rename(&file, root.join("renamed.tsv")).unwrap();
        assert_eq!(watch.poll(), Change::Changed);
        let mut watch = DirectoryWatch::new(&root).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(watch.poll(), Change::Changed);
        let mut watch = DirectoryWatch::new(&root).expect("watch existing ancestor");
        std::fs::create_dir(&root).unwrap();
        assert_eq!(watch.poll(), Change::Changed);
        std::fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn periodic_reconciliation() {
        let root =
            std::env::temp_dir().join(format!("waypinyin-watch-periodic-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut watch = DirectoryWatch::new(&root).unwrap();
        watch.reconcile_next();
        assert_eq!(watch.poll(), Change::Reconcile);
        std::fs::remove_dir(&root).unwrap();
    }
}
