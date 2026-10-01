//! `cargo xtask package`: the Cell as a Debian package (Phase 1 plan,
//! `16-alpha-release`).
//!
//! Builds `nomos-cell` as a static `musl` binary in release mode and
//! assembles, with `dpkg-deb`, a package holding the binary, a systemd
//! service that runs `enforce` on the Canon at `/etc/nomos/canon.cbor` with
//! the bundle beside it, a timer that starts the service two minutes after
//! boot and fifteen minutes after each run ends, and the maintainer
//! scripts that create the Cell's state directory, enable the timer, and
//! remove the state on purge. The package's version is the workspace's,
//! with the pre-release separator Debian orders before a release.

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

/// The target every package is built for.
pub(crate) const TARGET: &str = "x86_64-unknown-linux-musl";

/// The service the package installs. A run is a oneshot that is never
/// restarted, so systemd's start limit, five starts in ten seconds, would
/// only refuse an operator who starts it by hand in quick succession; it is
/// lifted.
pub(crate) const SERVICE: &str = "[Unit]
Description=Nomos Cell: converge this host to its Canon
Documentation=https://github.com/kmosoti/nomos
ConditionPathExists=/etc/nomos/canon.cbor
StartLimitIntervalSec=0
Wants=network-online.target
After=network-online.target

[Service]
Type=oneshot
ExecStart=/usr/bin/nomos-cell --state /var/lib/nomos enforce --canon /etc/nomos/canon.cbor --bundle /etc/nomos/bundle
";

/// The timer that starts it.
pub(crate) const TIMER: &str = "[Unit]
Description=Run the Nomos Cell periodically

[Timer]
OnBootSec=2min
OnUnitInactiveSec=15min

[Install]
WantedBy=timers.target
";

/// After unpacking: the state directory, and the timer enabled and
/// started when systemd runs.
pub(crate) const POSTINST: &str = "#!/bin/sh
set -e
if [ \"$1\" = configure ]; then
    install -d -m 0700 /var/lib/nomos
    if [ -d /run/systemd/system ]; then
        systemctl daemon-reload
        systemctl enable --now nomos-cell.timer
    fi
fi
";

/// Before removal: the timer stopped and disabled. An upgrade keeps it.
pub(crate) const PRERM: &str = "#!/bin/sh
set -e
if [ \"$1\" = remove ] && [ -d /run/systemd/system ]; then
    systemctl disable --now nomos-cell.timer || true
fi
";

/// After removal: systemd told, and on purge the Cell's state removed. The
/// operator's Canon and bundle under /etc/nomos are theirs and stay.
pub(crate) const POSTRM: &str = "#!/bin/sh
set -e
if [ -d /run/systemd/system ]; then
    systemctl daemon-reload || true
fi
if [ \"$1\" = purge ]; then
    rm -rf /var/lib/nomos
fi
";

/// The Debian form of a Cargo version: a pre-release sorts before its
/// release with `~`, where Cargo uses `-`.
pub(crate) fn debian_version(cargo: &str) -> String {
    match cargo.split_once('-') {
        Some((release, pre)) => format!("{release}~{pre}"),
        None => cargo.to_string(),
    }
}

/// The package's control file.
pub(crate) fn control(version: &str, installed_kib: u64) -> String {
    format!(
        "Package: nomos-cell
Version: {version}
Architecture: amd64
Maintainer: Nomos <nomos@users.noreply.github.com>
Installed-Size: {installed_kib}
Section: admin
Priority: optional
Homepage: https://github.com/kmosoti/nomos
Description: host-state convergence for Debian
 The Nomos Cell assesses this host against a compiled Canon, plans the
 Actions its Variances need, applies them through native interfaces,
 verifies the result, and records every step. Unknown evidence is never
 treated as noncompliance.
"
    )
}

fn ok(cmd: &mut Command) -> Result<String, String> {
    let out = cmd.output().map_err(|e| format!("{cmd:?}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "{cmd:?} exited {}: {}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Builds the static release binary and returns its path.
pub(crate) fn binary(root: &Path) -> Result<PathBuf, String> {
    let messages = ok(Command::new("cargo").current_dir(root).args([
        "build",
        "--release",
        "--locked",
        "--message-format=json",
        "--target",
        TARGET,
        "-p",
        "nomos-cell",
        "--bin",
        "nomos-cell",
    ]))?;
    messages
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|m| m["reason"] == "compiler-artifact" && m["target"]["name"] == "nomos-cell")
        .find_map(|m| m["executable"].as_str().map(PathBuf::from))
        .ok_or_else(|| "cargo built no nomos-cell executable".to_string())
}

fn write(path: &Path, text: &str, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Assembles the package from `binary` into `out`, at `version` (Debian
/// form), and returns its path.
pub(crate) fn assemble(
    root: &Path,
    binary: &Path,
    out: &Path,
    version: &str,
) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    let stage = out.join(format!("stage-{version}"));
    let _ = std::fs::remove_dir_all(&stage);
    let bin = stage.join("usr/bin/nomos-cell");
    std::fs::create_dir_all(bin.parent().ok_or("usr/bin")?).map_err(|e| e.to_string())?;
    std::fs::copy(binary, &bin).map_err(|e| format!("{}: {e}", binary.display()))?;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| e.to_string())?;
    write(
        &stage.join("usr/lib/systemd/system/nomos-cell.service"),
        SERVICE,
        0o644,
    )?;
    write(
        &stage.join("usr/lib/systemd/system/nomos-cell.timer"),
        TIMER,
        0o644,
    )?;
    std::fs::create_dir_all(stage.join("etc/nomos/bundle")).map_err(|e| e.to_string())?;
    let license = std::fs::read_to_string(root.join("LICENSE")).map_err(|e| e.to_string())?;
    write(
        &stage.join("usr/share/doc/nomos-cell/copyright"),
        &format!(
            "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/\n\
             Upstream-Name: nomos\nSource: https://github.com/kmosoti/nomos\n\n\
             Files: *\nLicense: Apache-2.0\n\n{license}"
        ),
        0o644,
    )?;
    let size = std::fs::metadata(&bin).map_err(|e| e.to_string())?.len();
    write(
        &stage.join("DEBIAN/control"),
        &control(version, size.div_ceil(1024) + 8),
        0o644,
    )?;
    write(&stage.join("DEBIAN/postinst"), POSTINST, 0o755)?;
    write(&stage.join("DEBIAN/prerm"), PRERM, 0o755)?;
    write(&stage.join("DEBIAN/postrm"), POSTRM, 0o755)?;
    let deb = out.join(format!("nomos-cell_{version}_amd64.deb"));
    ok(Command::new("dpkg-deb")
        .args(["--build", "--root-owner-group"])
        .arg(&stage)
        .arg(&deb))?;
    let _ = std::fs::remove_dir_all(&stage);
    Ok(deb)
}

/// The workspace's version, from `[workspace.package]` in the root
/// manifest.
pub(crate) fn workspace_version(root: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| e.to_string())?;
    let manifest: toml::Value = toml::from_str(&text).map_err(|e| e.to_string())?;
    manifest["workspace"]["package"]["version"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "Cargo.toml has no workspace.package.version".into())
}

/// The package at the workspace's version and one at a later version with
/// the same binary, for an install and an upgrade, built into `out`.
pub(crate) fn base_and_upgrade(root: &Path, out: &Path) -> Result<(PathBuf, PathBuf), String> {
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let version = debian_version(&workspace_version(root)?);
    let binary = binary(root)?;
    let base = assemble(root, &binary, out, &version)?;
    let upgrade = assemble(root, &binary, out, &format!("{version}+upgrade1"))?;
    Ok((base, upgrade))
}

/// Writes `SHA256SUMS` beside `deb`: the digest `sha256sum --check` reads,
/// of the package as it was built.
pub(crate) fn write_checksums(deb: &Path) -> Result<PathBuf, String> {
    let dir = deb.parent().ok_or("a package has a directory")?;
    let name = deb
        .file_name()
        .ok_or("a package has a name")?
        .to_string_lossy();
    let sums = dir.join("SHA256SUMS");
    std::fs::write(&sums, format!("{}  {name}\n", sha256(deb)?))
        .map_err(|e| format!("{}: {e}", sums.display()))?;
    Ok(sums)
}

/// The one file in `dir` named `nomos-cell_*_amd64.deb`.
fn only_deb(dir: &Path) -> Result<PathBuf, String> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().is_some_and(|n| {
                let n = n.to_string_lossy();
                n.starts_with("nomos-cell_") && n.ends_with("_amd64.deb")
            })
        })
        .collect();
    found.sort();
    match found.len() {
        1 => Ok(found.remove(0)),
        n => Err(format!(
            "{} holds {n} packages named nomos-cell_*_amd64.deb; it must hold exactly one",
            dir.display()
        )),
    }
}

/// The package and its upgrade fixture built once by `cargo xtask package
/// --upgrade-fixture`, in `dir` and `dir/upgrade`. The package must have the
/// digest `SHA256SUMS` beside it records, and `expect`, when given: the
/// digest the build that made it reported.
pub(crate) fn prebuilt(dir: &Path, expect: Option<&str>) -> Result<(PathBuf, PathBuf), String> {
    let base = only_deb(dir)?;
    let upgrade = only_deb(&dir.join("upgrade"))?;
    let digest = sha256(&base)?;
    let sums = std::fs::read_to_string(dir.join("SHA256SUMS"))
        .map_err(|e| format!("{}: {e}", dir.join("SHA256SUMS").display()))?;
    let name = base
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    if !sums.lines().any(|l| l == format!("{digest}  {name}")) {
        return Err(format!(
            "{name} is not the package SHA256SUMS records: its digest is {digest}"
        ));
    }
    if let Some(expected) = expect
        && expected != digest
    {
        return Err(format!(
            "{name} has digest {digest}, not the {expected} the build reported: it was replaced after it was built"
        ));
    }
    Ok((base, upgrade))
}

/// The SHA-256 of a file, in hex.
pub(crate) fn sha256(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A package the Debian suites install must be the one the build wrote
    /// and reported: replaced, or reported differently, it is refused.
    #[test]
    fn a_package_that_is_not_the_one_built_is_refused() {
        let dir = crate::scratch::dir("package", "prebuilt");
        let upgrade = dir.join("upgrade");
        std::fs::create_dir_all(&upgrade).unwrap();
        let base = dir.join("nomos-cell_1.0.0_amd64.deb");
        std::fs::write(&base, b"built").unwrap();
        std::fs::write(upgrade.join("nomos-cell_1.0.0+upgrade1_amd64.deb"), b"up").unwrap();
        write_checksums(&base).unwrap();
        let digest = sha256(&base).unwrap();
        assert!(prebuilt(&dir, Some(digest.as_str())).is_ok());
        assert!(prebuilt(&dir, None).is_ok());
        let wrong = "0".repeat(64);
        assert!(
            prebuilt(&dir, Some(wrong.as_str()))
                .unwrap_err()
                .contains("replaced")
        );
        std::fs::write(&base, b"replaced after the build").unwrap();
        assert!(
            prebuilt(&dir, None)
                .unwrap_err()
                .contains("not the package SHA256SUMS records")
        );
        std::fs::remove_file(dir.join("SHA256SUMS")).unwrap();
        assert!(prebuilt(&dir, None).is_err());
    }

    #[test]
    fn a_cargo_pre_release_sorts_before_its_release_in_debian() {
        assert_eq!(debian_version("0.1.0-alpha.1"), "0.1.0~alpha.1");
        assert_eq!(debian_version("0.1.0"), "0.1.0");
        assert_eq!(debian_version("1.2.3-rc.1+x"), "1.2.3~rc.1+x");
    }

    /// The service runs what the Cell's commands document, and the scripts
    /// keep the operator's Canon on purge.
    #[test]
    fn the_units_and_scripts_say_what_the_package_promises() {
        assert!(SERVICE.contains(
            "ExecStart=/usr/bin/nomos-cell --state /var/lib/nomos enforce --canon /etc/nomos/canon.cbor --bundle /etc/nomos/bundle"
        ));
        assert!(SERVICE.contains("ConditionPathExists=/etc/nomos/canon.cbor"));
        assert!(SERVICE.contains("StartLimitIntervalSec=0"));
        assert!(!SERVICE.contains("Restart="));
        assert!(TIMER.contains("WantedBy=timers.target"));
        assert!(POSTRM.contains("rm -rf /var/lib/nomos"));
        assert!(!POSTRM.contains("/etc/nomos"));
        let c = control("0.1.0~alpha.1", 5000);
        assert!(
            c.starts_with("Package: nomos-cell\nVersion: 0.1.0~alpha.1\nArchitecture: amd64\n")
        );
    }
}
