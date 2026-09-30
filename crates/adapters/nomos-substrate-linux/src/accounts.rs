//! Account names and numeric IDs, from the user and group databases beneath
//! the adapter's root ([substrate-contract.md], Files and Directories on
//! Linux).
//!
//! The files are read at use, through the same resolution as every other
//! path, and never through the C library's name service: the root may be a
//! scratch directory in tests, and Nomos reads what the host's files say,
//! not what a configured service answers.
//!
//! [substrate-contract.md]: ../../../../docs/formal/substrate-contract.md

use std::io::Read;
use std::os::fd::BorrowedFd;

use rustix::fs::{Mode, OFlags};
use rustix::io::Errno;

use super::RESOLVE;

/// The names and IDs of `/etc/passwd` and `/etc/group`, in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Accounts {
    users: Vec<(String, u32)>,
    groups: Vec<(String, u32)>,
    entries: Vec<User>,
}

/// One well-formed entry of `/etc/passwd`:
/// `name:password:uid:gid:gecos:home:shell`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    /// The account's name.
    pub name: String,
    /// Its numeric ID.
    pub uid: u32,
    /// Its primary group's numeric ID.
    pub gid: u32,
    /// Its home directory, as recorded.
    pub home: String,
    /// Its login shell, as recorded.
    pub shell: String,
}

/// The well-formed entries of a user database. A line with fewer than
/// seven fields, or a non-numeric ID, is skipped.
fn users(text: &str) -> Vec<User> {
    text.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            if f.len() < 7 || f[0].is_empty() {
                return None;
            }
            Some(User {
                name: f[0].to_string(),
                uid: f[2].parse().ok()?,
                gid: f[3].parse().ok()?,
                home: f[5].to_string(),
                shell: f[6..].join(":"),
            })
        })
        .collect()
}

/// The name and numeric ID of each well-formed line: `name:password:id:...`.
/// A line that is empty, a comment, or not of that form is skipped.
fn entries(text: &str) -> Vec<(String, u32)> {
    text.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut fields = l.split(':');
            let name = fields.next()?;
            let _password = fields.next()?;
            let id = fields.next()?.parse().ok()?;
            (!name.is_empty()).then(|| (name.to_string(), id))
        })
        .collect()
}

fn read(root: BorrowedFd<'_>, path: &str) -> Result<String, Errno> {
    let fd = match rustix::fs::openat2(
        root,
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
        RESOLVE,
    ) {
        Ok(fd) => fd,
        Err(Errno::NOENT) => return Ok(String::new()),
        Err(e) => return Err(e),
    };
    let mut text = String::new();
    std::fs::File::from(fd)
        .read_to_string(&mut text)
        .map_err(|e| e.raw_os_error().map_or(Errno::IO, Errno::from_raw_os_error))?;
    Ok(text)
}

impl Accounts {
    /// The databases beneath `root`. A missing file is an empty database.
    pub fn load(root: BorrowedFd<'_>) -> Result<Self, Errno> {
        Ok(Accounts::parse(
            &read(root, "etc/passwd")?,
            &read(root, "etc/group")?,
        ))
    }

    /// The databases from their text.
    pub fn parse(passwd: &str, group: &str) -> Self {
        Accounts {
            users: entries(passwd),
            groups: entries(group),
            entries: users(passwd),
        }
    }

    /// The user ID of `name`, from its first entry.
    pub fn uid(&self, name: &str) -> Option<u32> {
        self.users
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, id)| *id)
    }

    /// The group ID of `name`, from its first entry.
    pub fn gid(&self, name: &str) -> Option<u32> {
        self.groups
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, id)| *id)
    }

    /// The first full entry of `name`.
    pub fn user(&self, name: &str) -> Option<&User> {
        self.entries.iter().find(|u| u.name == name)
    }

    /// How many entries of the user database hold `uid`.
    pub fn holders(&self, uid: u32) -> usize {
        self.entries.iter().filter(|u| u.uid == uid).count()
    }

    /// The first name the user database gives `uid`.
    pub fn user_name(&self, uid: u32) -> Option<&str> {
        self.users
            .iter()
            .find(|(_, id)| *id == uid)
            .map(|(n, _)| n.as_str())
    }

    /// The first name the group database gives `gid`.
    pub fn group_name(&self, gid: u32) -> Option<&str> {
        self.groups
            .iter()
            .find(|(_, id)| *id == gid)
            .map(|(n, _)| n.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::Accounts;

    #[test]
    fn names_and_ids_come_from_the_first_well_formed_entry() {
        let a = Accounts::parse(
            "# comment\nroot:x:0:0:root:/root:/bin/bash\n\nbad line\nno-id:x:abc:0\nalias:x:0:0\napp:x:1001:1001::/home/app:/bin/sh\n",
            "root:x:0:\nadm:x:4:syslog\napp:x:1001:\n:x:7:\n",
        );
        assert_eq!(a.uid("root"), Some(0));
        assert_eq!(a.uid("app"), Some(1001));
        assert_eq!(a.uid("no-id"), None);
        assert_eq!(a.user_name(0), Some("root"));
        assert_eq!(a.user_name(4242), None);
        assert_eq!(a.gid("adm"), Some(4));
        assert_eq!(a.group_name(7), None);
    }

    #[test]
    fn a_full_entry_has_seven_fields_and_numeric_ids() {
        let a = Accounts::parse(
            "root:x:0:0:root:/root:/bin/bash\nshort:x:5:5\nbad:x:1:g::/:/bin/sh\nalias:x:0:0::/:/bin/sh\n",
            "",
        );
        let root = a.user("root").unwrap();
        assert_eq!((root.uid, root.gid), (0, 0));
        assert_eq!(
            (root.home.as_str(), root.shell.as_str()),
            ("/root", "/bin/bash")
        );
        assert_eq!(a.user("short"), None);
        assert_eq!(a.user("bad"), None);
        assert_eq!(a.holders(0), 2);
        assert_eq!(a.holders(5), 0);
    }
}
