//! XDG user directories. Empty and relative XDG values are ignored.
use std::path::{Path, PathBuf};

fn base(
    xdg: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
    fallback: &str,
) -> Option<PathBuf> {
    xdg.map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            home.map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(fallback))
        })
}
pub(crate) fn data() -> Option<PathBuf> {
    base(
        std::env::var_os("XDG_DATA_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
        ".local/share",
    )
    .map(|p| p.join("waypinyin"))
}
pub(crate) fn config() -> Option<PathBuf> {
    base(
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
        ".config",
    )
    .map(|p| p.join("waypinyin"))
}

pub(crate) fn migrate(old: &Path, new: &Path) -> std::io::Result<()> {
    use std::io::Write;
    if old == new || new.try_exists()? || !old.try_exists()? {
        return Ok(());
    }
    let bytes = std::fs::read(old)?;
    std::fs::create_dir_all(
        new.parent()
            .ok_or_else(|| std::io::Error::other("missing parent"))?,
    )?;
    // Publish a complete file atomically; hard_link refuses to overwrite a
    // concurrently created destination. The temporary file is on the same FS.
    let temporary = new.with_extension(format!(
        "migration-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let result = (|| {
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        out.write_all(&bytes)?;
        out.sync_all()?;
        match std::fs::hard_link(&temporary, new) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(e) => Err(e),
        }
    })();
    let _ = std::fs::remove_file(&temporary);
    result?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn xdg_absolute_and_fallback() {
        assert_eq!(
            base(
                Some("/data".as_ref()),
                Some("/home/user".as_ref()),
                ".local/share"
            ),
            Some("/data".into())
        );
        for bad in ["", "relative"] {
            assert_eq!(
                base(
                    Some(bad.as_ref()),
                    Some("/home/user".as_ref()),
                    ".local/share"
                ),
                Some("/home/user/.local/share".into())
            );
        }
        assert_eq!(base(None, None, ".config"), None);
    }
    #[test]
    fn migration_preserves_both_files() {
        let dir = std::env::temp_dir().join(format!("waypinyin-migration-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("old");
        let new = dir.join("new/user.txt");
        std::fs::write(&old, b"learning").unwrap();
        migrate(&old, &new).unwrap();
        assert_eq!(std::fs::read(&new).unwrap(), b"learning");
        std::fs::write(&old, b"changed").unwrap();
        migrate(&old, &new).unwrap();
        assert_eq!(std::fs::read(&new).unwrap(), b"learning");
        assert_eq!(std::fs::read(&old).unwrap(), b"changed");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn migration_failure_leaves_source_and_no_destination() {
        let dir = std::env::temp_dir().join(format!(
            "waypinyin-migration-failure-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("old");
        let parent = dir.join("not-a-directory");
        std::fs::write(&old, b"learning").unwrap();
        std::fs::write(&parent, b"file").unwrap();
        assert!(migrate(&old, &parent.join("user.txt")).is_err());
        assert_eq!(std::fs::read(&old).unwrap(), b"learning");
        assert!(!parent.join("user.txt").exists());
        assert!(migrate(&dir.join("missing"), &dir.join("new")).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
