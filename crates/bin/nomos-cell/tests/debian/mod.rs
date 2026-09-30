//! Helpers for the tests that change a real Debian host: a local
//! repository of dummy packages, and the ground truth of a package read
//! with `dpkg-query`.

#![allow(dead_code)]

use std::path::Path;
use std::process::Command;

use nomos_core::condition::PackageVersion;
use nomos_core::observation::PackageEvidence;

/// The local repository the tests build.
pub const REPO: &str = "/srv/nomos-repo";

pub fn run(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .env("DEBIAN_FRONTEND", "noninteractive")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap()
        .status
        .success()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Builds the dummy package `name` at `version`, depending on `depends`,
/// into the repository, and returns its file.
pub fn build(name: &str, version: &str, depends: Option<&str>) -> String {
    let file = format!("{REPO}/{name}_{version}_all.deb");
    if Path::new(&file).exists() {
        return file;
    }
    let dir = format!("/tmp/nomos-deb-{name}-{version}");
    std::fs::create_dir_all(format!("{dir}/DEBIAN")).unwrap();
    let depends = depends.map_or(String::new(), |d| format!("Depends: {d}\n"));
    std::fs::write(
        format!("{dir}/DEBIAN/control"),
        format!(
            "Package: {name}\nVersion: {version}\nArchitecture: all\n{depends}\
             Maintainer: Nomos Tests <tests@nomos.invalid>\nDescription: Nomos test package\n"
        ),
    )
    .unwrap();
    std::fs::create_dir_all(REPO).unwrap();
    assert!(run(
        "dpkg-deb",
        &["--build", "--root-owner-group", &dir, &file]
    ));
    file
}

/// Writes the repository's index from the packages in it, and reads it
/// into apt's lists, from this source alone.
pub fn publish() {
    let mut index = String::new();
    let mut debs: Vec<_> = std::fs::read_dir(REPO)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "deb"))
        .collect();
    debs.sort();
    for deb in debs {
        let control = Command::new("dpkg-deb")
            .arg("-f")
            .arg(&deb)
            .output()
            .unwrap();
        let bytes = std::fs::read(&deb).unwrap();
        index.push_str(&String::from_utf8(control.stdout).unwrap());
        index.push_str(&format!(
            "Filename: ./{}\nSize: {}\nSHA256: {}\n\n",
            deb.file_name().unwrap().to_string_lossy(),
            bytes.len(),
            hex(&nomos_canon::sha256::digest(&bytes))
        ));
    }
    std::fs::write(format!("{REPO}/Packages"), index).unwrap();
    std::fs::write(
        "/etc/apt/sources.list.d/nomos-test.list",
        format!("deb [trusted=yes] file:{REPO} ./\n"),
    )
    .unwrap();
    assert!(run(
        "apt-get",
        &[
            "update",
            "-o",
            "Dir::Etc::sourcelist=sources.list.d/nomos-test.list",
            "-o",
            "Dir::Etc::sourceparts=-",
            "-o",
            "APT::Get::List-Cleanup=0",
        ]
    ));
}

/// The ground truth of a package, read with `dpkg-query`: `None` when
/// dpkg holds no installed or broken state for it.
pub fn package_truth(name: &str) -> Option<PackageEvidence> {
    let out = Command::new("dpkg-query")
        .args(["-W", "-f=${db:Status-Abbrev}|${Version}", name])
        .output()
        .unwrap();
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).unwrap();
    let (abbrev, version) = text.split_once('|').unwrap();
    match abbrev.trim() {
        "ii" => Some(PackageEvidence::Installed {
            version: PackageVersion::new(version).unwrap(),
        }),
        "un" | "rc" | "pn" => None,
        _ => Some(PackageEvidence::Broken),
    }
}

/// Makes dpkg's status database readable again, whatever a denial did.
pub fn readable_status() {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        "/var/lib/dpkg/status",
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
}
