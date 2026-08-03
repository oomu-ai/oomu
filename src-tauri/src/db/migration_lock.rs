use super::{io_to_sql_error, migration_recovery_error, set_private_file};
use std::{fs, io, path::Path};

/// Holds an advisory lock for the complete schema-migration protocol.
///
/// SQLite serializes individual write transactions, but migration backup and
/// preflight work also has to be serialized when a development rebuild briefly
/// overlaps the process it is replacing.
pub(super) struct MigrationFileLock {
    file: fs::File,
}

impl MigrationFileLock {
    pub(super) fn acquire(database_path: &Path) -> rusqlite::Result<Self> {
        let parent = database_path.parent().ok_or_else(|| {
            migration_recovery_error("database path has no parent for migration lock")
        })?;
        fs::create_dir_all(parent).map_err(io_to_sql_error)?;
        let database_name = database_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| migration_recovery_error("database path has no migration lock name"))?;
        let lock_path = parent.join(format!(".{database_name}.migration.lock"));

        #[cfg(unix)]
        let file = {
            use std::os::unix::fs::OpenOptionsExt;
            fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&lock_path)
                .map_err(io_to_sql_error)?
        };
        #[cfg(not(unix))]
        let file = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(io_to_sql_error)?;

        set_private_file(&lock_path)?;

        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            loop {
                if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                    break;
                }
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::EINTR) {
                    return Err(io_to_sql_error(error));
                }
            }
        }

        Ok(Self { file })
    }
}

impl Drop for MigrationFileLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
        }
    }
}
