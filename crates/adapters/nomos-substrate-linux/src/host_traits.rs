//! The host's Traits ([cell-commands.md], Traits): read from files beneath
//! the adapter's root, with the same resolution as every other path, and
//! never inferred. A source that cannot be read gives no Trait.
//!
//! [cell-commands.md]: ../../../../docs/formal/cell-commands.md

use std::io::Read;
use std::os::fd::BorrowedFd;

use nomos_core::observation::Instant;
use nomos_core::traits::{Stability, Trait};
use rustix::fs::{Mode, OFlags};

use crate::RESOLVE;
use crate::packages::native_architecture;

fn read(root: BorrowedFd<'_>, path: &str) -> Option<String> {
    let fd = rustix::fs::openat2(
        root,
        path,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        RESOLVE,
    )
    .ok()?;
    let mut text = String::new();
    std::fs::File::from(fd).read_to_string(&mut text).ok()?;
    Some(text)
}

/// The value of `key` in an os-release file, unquoted.
pub fn os_release_field(text: &str, key: &str) -> Option<String> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|v| !v.is_empty())
}

/// The value of `key` in `/proc/meminfo`, in kibibytes.
pub fn meminfo_field(text: &str, key: &str) -> Option<String> {
    text.lines()
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim() == key)
        .and_then(|(_, v)| v.split_whitespace().next())
        .map(str::to_string)
}

/// The Traits the host reports, and the keys whose source could not be
/// read.
pub fn traits(root: BorrowedFd<'_>, now: Instant) -> (Vec<Trait>, Vec<&'static str>) {
    let mut found = Vec::new();
    let mut missing = Vec::new();
    let mut add = |key: &'static str, value: Option<String>, source: &str, stability| match value {
        Some(value) => found.push(Trait {
            key: key.to_string(),
            value,
            source: source.to_string(),
            observed_at: now,
            stability,
        }),
        None => missing.push(key),
    };
    let first_line = |text: Option<String>| {
        text.and_then(|t| t.lines().next().map(str::trim).map(str::to_string))
            .filter(|t| !t.is_empty())
    };
    add(
        "machine.id",
        first_line(read(root, "etc/machine-id")),
        "/etc/machine-id",
        Stability::Identity,
    );
    // /etc/os-release is usually a link to /usr/lib/os-release, which the
    // resolution refuses to follow; the os-release format names the second
    // as the fallback.
    let (os, os_source) = match read(root, "etc/os-release") {
        Some(t) => (Some(t), "/etc/os-release"),
        None => (read(root, "usr/lib/os-release"), "/usr/lib/os-release"),
    };
    let field = |k| os.as_deref().and_then(|t| os_release_field(t, k));
    add("os.id", field("ID"), os_source, Stability::Stable);
    add(
        "os.version_id",
        field("VERSION_ID"),
        os_source,
        Stability::Stable,
    );
    add(
        "architecture",
        Some(native_architecture().to_string()),
        "build target",
        Stability::Stable,
    );
    add(
        "hostname",
        first_line(read(root, "proc/sys/kernel/hostname")),
        "/proc/sys/kernel/hostname",
        Stability::Stable,
    );
    add(
        "kernel.release",
        first_line(read(root, "proc/sys/kernel/osrelease")),
        "/proc/sys/kernel/osrelease",
        Stability::Dynamic,
    );
    let meminfo = read(root, "proc/meminfo");
    add(
        "memory.total",
        meminfo
            .as_deref()
            .and_then(|t| meminfo_field(t, "MemTotal")),
        "/proc/meminfo",
        Stability::Stable,
    );
    add(
        "memory.available",
        meminfo
            .as_deref()
            .and_then(|t| meminfo_field(t, "MemAvailable")),
        "/proc/meminfo",
        Stability::Ephemeral,
    );
    (found, missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_release_and_meminfo_fields_read_as_their_formats_say() {
        let os = "PRETTY_NAME=\"Debian GNU/Linux 12 (bookworm)\"\nID=debian\nVERSION_ID=\"12\"\nEMPTY=\n";
        assert_eq!(os_release_field(os, "ID").as_deref(), Some("debian"));
        assert_eq!(os_release_field(os, "VERSION_ID").as_deref(), Some("12"));
        assert_eq!(os_release_field(os, "EMPTY"), None);
        assert_eq!(os_release_field(os, "ABSENT"), None);
        let mem = "MemTotal:       16318156 kB\nMemFree:  1 kB\nMemAvailable:   12001234 kB\n";
        assert_eq!(meminfo_field(mem, "MemTotal").as_deref(), Some("16318156"));
        assert_eq!(
            meminfo_field(mem, "MemAvailable").as_deref(),
            Some("12001234")
        );
        assert_eq!(meminfo_field(mem, "SwapTotal"), None);
    }
}
