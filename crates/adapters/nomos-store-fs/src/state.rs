//! Ownership and safety of the Cell's state directory
//! ([cell-commands.md](../../../../docs/formal/cell-commands.md), State
//! Directory).
//!
//! [`StateLease`] is mutation authority: an exclusive advisory lock on the
//! file `lock` in the state directory, taken without waiting and held for as
//! long as the value lives. The lock belongs to the open file, so the kernel
//! releases it when the process dies, however it dies.
//!
//! [`check`] is the safety rule every object under the state directory must
//! meet before it is trusted: the right type, not a link, owned by the
//! process's effective user, and with no permission bit for group or others.
//! It is applied to the directory, to `lock`, to the journal, to the content
//! store, and to every blob read from it.

use std::fmt;
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};

use rustix::fs::{FileType, FlockOperation, Mode, OFlags};
use rustix::io::Errno;

/// Why the state directory cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateError {
    /// Another process holds mutation authority over this directory.
    Held(PathBuf),
    /// An object under the state directory is not private to the Cell.
    Unsafe {
        /// The object.
        path: PathBuf,
        /// What is wrong with it.
        why: &'static str,
    },
    /// The directory could not be read or created.
    Io(String),
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateError::Held(dir) => write!(
                f,
                "state directory {} is already in use by another nomos-cell",
                dir.display()
            ),
            StateError::Unsafe { path, why } => {
                write!(f, "{}: unsafe state ({why})", path.display())
            }
            StateError::Io(e) => write!(f, "state: {e}"),
        }
    }
}

impl std::error::Error for StateError {}

fn io(path: &Path, e: impl fmt::Display) -> StateError {
    StateError::Io(format!("{}: {e}", path.display()))
}

/// What an object under the state directory must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Directory,
    File,
}

/// Refuses `fd`, which was opened at `path` without following a link,
/// unless it is the right kind of object, owned by the effective user, and
/// closed to group and others.
pub(crate) fn check(fd: impl AsFd, path: &Path, kind: Kind) -> Result<(), StateError> {
    let stat = rustix::fs::fstat(fd).map_err(|e| io(path, e))?;
    let unsafe_ = |why| {
        Err(StateError::Unsafe {
            path: path.to_path_buf(),
            why,
        })
    };
    let (want, why) = match kind {
        Kind::Directory => (FileType::Directory, "not a directory"),
        Kind::File => (FileType::RegularFile, "not a regular file"),
    };
    if FileType::from_raw_mode(stat.st_mode as _) != want {
        return unsafe_(why);
    }
    if stat.st_uid != rustix::process::geteuid().as_raw() {
        return unsafe_("not owned by the user running the Cell");
    }
    if stat.st_mode & 0o077 != 0 {
        return unsafe_("accessible to group or others");
    }
    Ok(())
}

/// Whether `errno` is what opening a link without following it reports.
pub(crate) fn is_link(errno: Errno) -> bool {
    errno == Errno::LOOP || errno == Errno::NOTDIR
}

/// The state directory `dir`, opened without following a link and checked.
/// With `create`, the last component is created, with mode `0700` whatever
/// the umask, and the ones above it are created as needed; without it, an
/// absent directory is `None`.
pub(crate) fn open_root(dir: &Path, create: bool) -> Result<Option<OwnedFd>, StateError> {
    let mut made = false;
    if create {
        if let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
        }
        match rustix::fs::mkdir(dir, Mode::from_raw_mode(0o700)) {
            Ok(()) => made = true,
            Err(Errno::EXIST) => {}
            Err(e) => return Err(io(dir, e)),
        }
    }
    let fd = match rustix::fs::open(
        dir,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(Errno::NOENT) if !create => return Ok(None),
        Err(e) if is_link(e) => {
            return Err(StateError::Unsafe {
                path: dir.to_path_buf(),
                why: "not a directory, or a symbolic link",
            });
        }
        Err(e) => return Err(io(dir, e)),
    };
    if made {
        rustix::fs::fchmod(&fd, Mode::from_raw_mode(0o700)).map_err(|e| io(dir, e))?;
    }
    check(&fd, dir, Kind::Directory)?;
    Ok(Some(fd))
}

/// Refuses the state directory `dir` if it exists and is not safe. Returns
/// whether it exists. Creates nothing: this is the check of the read-only
/// commands.
pub fn inspect(dir: &Path) -> Result<bool, StateError> {
    Ok(open_root(dir, false)?.is_some())
}

/// Mutation authority over one state directory, held until it is dropped.
#[derive(Debug)]
pub struct StateLease {
    // The lock is released when this file is closed.
    _lock: OwnedFd,
    dir: PathBuf,
}

impl StateLease {
    /// Takes mutation authority over `dir`, creating and checking it first.
    /// Never waits: a directory another process holds is
    /// [`StateError::Held`].
    pub fn acquire(dir: &Path) -> Result<StateLease, StateError> {
        let root = open_root(dir, true)?.ok_or_else(|| io(dir, "absent after creation"))?;
        let path = dir.join("lock");
        let lock = match rustix::fs::openat(
            root.as_fd(),
            "lock",
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        ) {
            Ok(fd) => fd,
            Err(e) if is_link(e) => {
                return Err(StateError::Unsafe {
                    path,
                    why: "a symbolic link",
                });
            }
            Err(e) => return Err(io(&path, e)),
        };
        check(&lock, &path, Kind::File)?;
        match rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => Ok(StateLease {
                _lock: lock,
                dir: dir.to_path_buf(),
            }),
            Err(Errno::WOULDBLOCK) => Err(StateError::Held(dir.to_path_buf())),
            Err(e) => Err(io(&path, e)),
        }
    }

    /// The directory this lease is over.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// A lease excludes a second one over the same directory, in the same
    /// process or another, until the first is dropped.
    #[test]
    fn a_held_state_directory_is_refused_until_it_is_released() {
        let dir = scratch::path("lease", "held");
        let first = StateLease::acquire(&dir).unwrap();
        assert_eq!(first.dir(), dir);
        assert_eq!(
            StateLease::acquire(&dir).unwrap_err(),
            StateError::Held(dir.clone())
        );
        drop(first);
        StateLease::acquire(&dir).expect("released with its holder");
    }

    #[test]
    fn different_state_directories_are_independent() {
        let (a, b) = (scratch::path("lease", "a"), scratch::path("lease", "b"));
        let (_a, _b) = (
            StateLease::acquire(&a).unwrap(),
            StateLease::acquire(&b).unwrap(),
        );
    }

    /// A new state directory and lock file are private, whatever the
    /// process's umask is.
    #[test]
    fn a_new_state_directory_is_private() {
        let dir = scratch::path("lease", "new");
        let _lease = StateLease::acquire(&dir).unwrap();
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("lock")), 0o600);
    }

    #[test]
    fn a_state_directory_open_to_group_or_others_is_refused() {
        for bits in [0o750, 0o755, 0o770, 0o707, 0o701] {
            let dir = scratch::dir("lease", &format!("mode-{bits:o}"));
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(bits)).unwrap();
            let err = StateLease::acquire(&dir).unwrap_err();
            assert!(
                matches!(err, StateError::Unsafe { .. }),
                "{bits:o}: {err:?}"
            );
            assert_eq!(inspect(&dir).unwrap_err(), err, "{bits:o}");
            assert_eq!(mode(&dir), bits, "the mode is not repaired");
        }
    }

    #[test]
    fn a_state_directory_that_is_a_link_is_refused() {
        let real = scratch::dir("lease", "real");
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = scratch::path("lease", "link");
        symlink(&real, &link).unwrap();
        for result in [
            StateLease::acquire(&link).map(drop),
            inspect(&link).map(drop),
        ] {
            assert!(
                matches!(result, Err(StateError::Unsafe { .. })),
                "{result:?}"
            );
        }
    }

    #[test]
    fn a_lock_file_that_is_a_link_or_open_to_others_is_refused() {
        let dir = scratch::path("lease", "lockfile");
        drop(StateLease::acquire(&dir).unwrap());
        std::fs::set_permissions(dir.join("lock"), std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            StateLease::acquire(&dir),
            Err(StateError::Unsafe { .. })
        ));
        std::fs::remove_file(dir.join("lock")).unwrap();
        symlink("/dev/null", dir.join("lock")).unwrap();
        assert!(matches!(
            StateLease::acquire(&dir),
            Err(StateError::Unsafe { .. })
        ));
    }

    /// Only a root process can give a file away, and only it needs to be
    /// told it was given to someone else.
    #[test]
    fn a_state_directory_owned_by_another_user_is_refused() {
        if !rustix::process::geteuid().is_root() {
            return;
        }
        let dir = scratch::path("lease", "owner");
        drop(StateLease::acquire(&dir).unwrap());
        std::os::unix::fs::chown(&dir, Some(65534), None).unwrap();
        assert!(matches!(
            StateLease::acquire(&dir),
            Err(StateError::Unsafe { .. })
        ));
    }

    #[test]
    fn an_absent_state_directory_is_absent_to_a_reader_and_stays_so() {
        let dir = scratch::path("lease", "absent");
        assert_eq!(inspect(&dir), Ok(false));
        assert!(!dir.exists());
    }
}
