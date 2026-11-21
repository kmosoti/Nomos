//! `cargo xtask package`: Debian packaging and multi-artifact checksums for `nomos-loom`.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::package::{ok, sha256, write_file};

/// Debian package target for `nomos-loom` (links glibc for SQLite WAL and TLS roots).
pub(crate) const LOOM_TARGET: &str = "x86_64-unknown-linux-gnu";

/// Runtime package dependencies for `nomos-loom` on Debian 12 and 13.
pub(crate) const LOOM_DEPENDS: [&str; 2] = ["libc6", "systemd"];

/// Systemd service unit running headless `nomos-loom serve`.
pub(crate) const LOOM_SERVICE: &str = "[Unit]
Description=Nomos Loom: headless IPv6 fleet coordinator and lease broker
Documentation=https://github.com/kmosoti/nomos
Wants=network-online.target
After=network-online.target

[Service]
Type=simple
StateDirectory=nomos-loom
ExecStart=/usr/bin/nomos-loom serve --listen [::1]:7440 --quic-listen [::1]:7443 --db-path /var/lib/nomos-loom/leases.sqlite --vault-dir /var/lib/nomos/vault
Restart=on-failure
RestartSec=5s

[Install]
WantedBy=multi-user.target
";

/// Maintainer `postinst`: ensures state dir exists and enables `nomos-loom.service`.
pub(crate) const LOOM_POSTINST: &str = "#!/bin/sh
set -e
if [ \"$1\" = configure ]; then
    install -d -m 0750 /var/lib/nomos-loom
    if [ -d /run/systemd/system ]; then
        systemctl daemon-reload
        systemctl enable --now nomos-loom.service || true
    fi
fi
";

/// Maintainer `prerm`: stops and disables `nomos-loom.service` on package removal.
pub(crate) const LOOM_PRERM: &str = "#!/bin/sh
set -e
if [ \"$1\" = remove ] && [ -d /run/systemd/system ]; then
    systemctl disable --now nomos-loom.service || true
fi
";

/// Maintainer `postrm`: reloads systemd and removes `/var/lib/nomos-loom` on purge.
pub(crate) const LOOM_POSTRM: &str = "#!/bin/sh
set -e
if [ -d /run/systemd/system ]; then
    systemctl daemon-reload || true
fi
if [ \"$1\" = purge ]; then
    rm -rf /var/lib/nomos-loom
fi
";

/// Generates `DEBIAN/control` for `nomos-loom`.
pub(crate) fn loom_control(version: &str, installed_kib: u64) -> String {
    let depends = LOOM_DEPENDS.join(", ");
    format!(
        "Package: nomos-loom
Version: {version}
Architecture: amd64
Maintainer: Nomos <nomos@users.noreply.github.com>
Installed-Size: {installed_kib}
Depends: {depends}
Section: admin
Priority: optional
Homepage: https://github.com/kmosoti/nomos
Description: headless IPv6 fleet coordinator and PSI governor for Debian
 The Nomos Loom runs the fleet coordinator HTTP/SSE and pinned QUIC RPC
 endpoints, SQLite WAL credential lease broker, and deterministic PSI
 resource governor without bundling browser WebAssembly assets.
"
    )
}

/// Builds the release `nomos-loom` binary and returns its executable path.
pub(crate) fn loom_binary(root: &Path) -> Result<PathBuf, String> {
    let messages = ok(Command::new("cargo").current_dir(root).args([
        "build",
        "--release",
        "--locked",
        "--message-format=json",
        "--target",
        LOOM_TARGET,
        "-p",
        "nomos-loom",
        "--bin",
        "nomos-loom",
    ]))?;
    messages
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|m| m["reason"] == "compiler-artifact" && m["target"]["name"] == "nomos-loom")
        .find_map(|m| m["executable"].as_str().map(PathBuf::from))
        .ok_or_else(|| "cargo built no nomos-loom executable".to_string())
}

/// Assembles `nomos-loom_<version>_amd64.deb` inside `out` using `dpkg-deb`.
pub(crate) fn assemble_loom(
    root: &Path,
    binary: &Path,
    out: &Path,
    version: &str,
) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    let stage = out.join(format!("stage-loom-{version}"));
    let _ = std::fs::remove_dir_all(&stage);
    let bin = stage.join("usr/bin/nomos-loom");
    std::fs::create_dir_all(bin.parent().ok_or("usr/bin")?).map_err(|e| e.to_string())?;
    std::fs::copy(binary, &bin).map_err(|e| format!("{}: {e}", binary.display()))?;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| e.to_string())?;

    write_file(
        &stage.join("usr/lib/systemd/system/nomos-loom.service"),
        LOOM_SERVICE,
        0o644,
    )?;
    let license = std::fs::read_to_string(root.join("LICENSE")).map_err(|e| e.to_string())?;
    write_file(
        &stage.join("usr/share/doc/nomos-loom/copyright"),
        &format!(
            "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/\n\
             Upstream-Name: nomos\nSource: https://github.com/kmosoti/nomos\n\n\
             Files: *\nLicense: Apache-2.0\n\n{license}"
        ),
        0o644,
    )?;
    let size = std::fs::metadata(&bin).map_err(|e| e.to_string())?.len();
    write_file(
        &stage.join("DEBIAN/control"),
        &loom_control(version, size.div_ceil(1024) + 8),
        0o644,
    )?;
    write_file(&stage.join("DEBIAN/postinst"), LOOM_POSTINST, 0o755)?;
    write_file(&stage.join("DEBIAN/prerm"), LOOM_PRERM, 0o755)?;
    write_file(&stage.join("DEBIAN/postrm"), LOOM_POSTRM, 0o755)?;

    let deb = out.join(format!("nomos-loom_{version}_amd64.deb"));
    ok(Command::new("dpkg-deb")
        .args(["--build", "--root-owner-group"])
        .arg(&stage)
        .arg(&deb))?;
    let _ = std::fs::remove_dir_all(&stage);
    Ok(deb)
}

/// Writes `SHA256SUMS` covering all passed package files in sorted filename order.
pub(crate) fn write_checksums_multi(debs: &[&Path]) -> Result<PathBuf, String> {
    let first = debs.first().ok_or("at least one package is required")?;
    let dir = first.parent().ok_or("a package has a directory")?;
    let mut lines = Vec::with_capacity(debs.len());
    for deb in debs {
        let name = deb
            .file_name()
            .ok_or("a package has a name")?
            .to_string_lossy();
        lines.push((name.into_owned(), sha256(deb)?));
    }
    lines.sort_by(|a, b| a.0.cmp(&b.0));
    let mut content = String::new();
    for (name, digest) in lines {
        content.push_str(&format!("{digest}  {name}\n"));
    }
    let sums = dir.join("SHA256SUMS");
    std::fs::write(&sums, content).map_err(|e| format!("{}: {e}", sums.display()))?;
    Ok(sums)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::prebuilt;

    #[test]
    fn loom_units_control_and_multi_checksums_work_alongside_cell() {
        assert!(LOOM_SERVICE.contains("ExecStart=/usr/bin/nomos-loom serve"));
        assert!(LOOM_SERVICE.contains("StateDirectory=nomos-loom"));
        assert!(LOOM_POSTRM.contains("rm -rf /var/lib/nomos-loom"));
        let ctrl = loom_control("0.1.0~alpha.2", 8000);
        assert!(ctrl.starts_with("Package: nomos-loom\nVersion: 0.1.0~alpha.2\n"));
        assert!(ctrl.contains("\nDepends: libc6, systemd\n"));

        let dir = crate::scratch::dir("package-loom", "multi");
        let upgrade = dir.join("upgrade");
        std::fs::create_dir_all(&upgrade).unwrap();
        let cell = dir.join("nomos-cell_1.0.0_amd64.deb");
        let loom = dir.join("nomos-loom_1.0.0_amd64.deb");
        std::fs::write(&cell, b"cell-deb").unwrap();
        std::fs::write(&loom, b"loom-deb").unwrap();
        std::fs::write(upgrade.join("nomos-cell_1.0.0+upgrade1_amd64.deb"), b"up").unwrap();

        let sums_path = write_checksums_multi(&[cell.as_path(), loom.as_path()]).unwrap();
        let sums_text = std::fs::read_to_string(sums_path).unwrap();
        assert_eq!(sums_text.lines().count(), 2);
        assert!(prebuilt(&dir, Some(&sha256(&cell).unwrap())).is_ok());
    }
}
