//! The demonstration Canon of `15-debian-convergence`, a slice of spec
//! §56, and the ways a writer other than Nomos can break it: shared by the
//! convergence experiment and the installed package's test.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{build, publish};
use nomos_canon::artifact::{Profile, encode};
use nomos_canon::model::{Canon, CanonBuilder, RelationKind};
use nomos_core::condition::{
    AccountClass, Activity, Content, DirectoryCondition, Enablement, FileCondition, Metadata,
    PackageCondition, PackageVersion, UnitCondition, UserCondition,
};
use nomos_core::resource::{AccountName, Digest, Mode, ResourcePath};

pub const ACCOUNT: &str = "nomos-demo";
pub const STATE_DIR: &str = "/var/lib/nomos-demo";
pub const CONF_DIR: &str = "/etc/nomos-demo";
pub const CONF: &str = "/etc/nomos-demo/demo.conf";
pub const PACKAGE: &str = "nomos-demo-tool";
pub const SERVICE: &str = "nomos-demo.service";
pub const LOADED: &str = "/var/lib/nomos-demo/loaded.conf";
pub const DOMAIN: &str = "demo.nomos.example";
pub const CONFIG: &[u8] = b"greeting = hello\n";

pub fn digest(bytes: &[u8]) -> Digest {
    Digest::from_bytes(nomos_canon::sha256::digest(bytes))
}

pub fn path(p: &str) -> ResourcePath {
    ResourcePath::new(p).unwrap()
}

pub fn mode(m: &str) -> Mode {
    Mode::from_octal(m).unwrap()
}

/// The demonstration Canon.
pub fn demonstration() -> Canon {
    let meta = |owner: Option<&str>, bits: &str| Metadata {
        owner: owner.map(|o| AccountName::new(o).unwrap()),
        group: None,
        mode: Some(mode(bits)),
    };
    CanonBuilder::new("debian-demonstration")
        .user(
            ACCOUNT,
            UserCondition::Present {
                class: AccountClass::System,
                home: Some(path(STATE_DIR)),
                shell: Some(path("/usr/sbin/nologin")),
            },
        )
        .directory(
            STATE_DIR,
            DirectoryCondition::Present {
                metadata: meta(Some(ACCOUNT), "0750"),
            },
        )
        .directory(
            CONF_DIR,
            DirectoryCondition::Present {
                metadata: meta(Some("root"), "0755"),
            },
        )
        .file(
            CONF,
            FileCondition::Present {
                content: Content::Exactly(digest(CONFIG)),
                metadata: meta(Some("root"), "0644"),
            },
        )
        .sysctl("kernel.domainname", DOMAIN)
        .package(
            PACKAGE,
            PackageCondition::Installed {
                version: Some(PackageVersion::new("1.0-1").unwrap()),
            },
        )
        .unit(
            SERVICE,
            UnitCondition {
                activity: Activity::Active,
                enablement: Enablement::Enabled,
            },
        )
        .relate(
            &format!("user:{ACCOUNT}"),
            RelationKind::Requires,
            &format!("directory:{STATE_DIR}"),
        )
        .relate(
            &format!("directory:{CONF_DIR}"),
            RelationKind::Requires,
            &format!("file:{CONF}"),
        )
        .relate(
            &format!("directory:{STATE_DIR}"),
            RelationKind::Requires,
            &format!("unit:{SERVICE}"),
        )
        .relate(
            &format!("package:{PACKAGE}"),
            RelationKind::Requires,
            &format!("unit:{SERVICE}"),
        )
        .relate(
            &format!("file:{CONF}"),
            RelationKind::OnChange,
            &format!("unit:{SERVICE}"),
        )
        .build()
        .unwrap()
}

pub fn sh(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .env("DEBIAN_FRONTEND", "noninteractive")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap()
        .status
        .success()
}

/// A resource of the demonstration, for the starting states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resource {
    User,
    StateDir,
    ConfDir,
    Conf,
    Sysctl,
    Package,
    Unit,
}

pub const RESOURCES: [Resource; 7] = [
    Resource::User,
    Resource::StateDir,
    Resource::ConfDir,
    Resource::Conf,
    Resource::Sysctl,
    Resource::Package,
    Resource::Unit,
];

/// What a starting state holds of one resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holds {
    /// None of it: no account, no directory, no file, a package not
    /// installed, a service stopped after failing. A kernel parameter has
    /// no absence and holds another value.
    Absent,
    /// It, with something the Canon does not ask for.
    Wrong,
}

/// The service's unit file, which the Canon does not manage: it runs as
/// the account and copies the configuration into its state directory
/// when it starts, with no limit on how often it may start.
pub const UNIT_FILE: &str = concat!(
    "[Unit]\n",
    "Description=Nomos demonstration service\n",
    // The experiment restarts the service more often than systemd's
    // default of five starts in ten seconds allows; a real service that
    // hits its limit fails its Action until the limit's window passes.
    "StartLimitIntervalSec=0\n",
    "[Service]\n",
    "User=nomos-demo\n",
    "ExecStartPre=+/bin/cp /etc/nomos-demo/demo.conf /var/lib/nomos-demo/loaded.conf\n",
    "ExecStart=/bin/sleep infinity\n",
    "[Install]\n",
    "WantedBy=multi-user.target\n",
);

/// Makes one resource hold `holds`, as a writer other than Nomos would.
pub fn perturb(resource: Resource, holds: Holds) {
    match (resource, holds) {
        (Resource::User, Holds::Absent) => {
            // An account in use cannot be deleted: the service stops first.
            sh("systemctl", &["stop", SERVICE]);
            sh("userdel", &[ACCOUNT]);
        }
        (Resource::User, Holds::Wrong) => {
            if !sh("usermod", &["--shell", "/bin/sh", ACCOUNT]) {
                assert!(sh(
                    "useradd",
                    &[
                        "--system",
                        "--no-create-home",
                        "--home-dir",
                        STATE_DIR,
                        "--shell",
                        "/bin/sh",
                        ACCOUNT
                    ]
                ));
            }
        }
        (Resource::StateDir, Holds::Absent) => {
            sh("systemctl", &["stop", SERVICE]);
            let _ = std::fs::remove_dir_all(STATE_DIR);
        }
        (Resource::StateDir, Holds::Wrong) => {
            std::fs::create_dir_all(STATE_DIR).unwrap();
            sh("chown", &["root:root", STATE_DIR]);
            sh("chmod", &["0777", STATE_DIR]);
        }
        (Resource::ConfDir, Holds::Absent) => {
            let _ = std::fs::remove_dir_all(CONF_DIR);
        }
        (Resource::ConfDir, Holds::Wrong) => {
            std::fs::create_dir_all(CONF_DIR).unwrap();
            sh("chmod", &["0700", CONF_DIR]);
        }
        (Resource::Conf, Holds::Absent) => {
            let _ = std::fs::remove_file(CONF);
        }
        (Resource::Conf, Holds::Wrong) => {
            std::fs::create_dir_all(CONF_DIR).unwrap();
            std::fs::write(CONF, b"greeting = goodbye\n").unwrap();
        }
        (Resource::Sysctl, _) => {
            std::fs::write("/proc/sys/kernel/domainname", "other.example\n").unwrap();
        }
        (Resource::Package, Holds::Absent) => {
            sh("dpkg", &["--purge", PACKAGE]);
        }
        (Resource::Package, Holds::Wrong) => {
            assert!(sh("dpkg", &["--install", &build(PACKAGE, "2.0-1", None)]));
        }
        (Resource::Unit, Holds::Absent) => {
            sh("systemctl", &["start", SERVICE]);
            sh("systemctl", &["kill", "--signal=KILL", SERVICE]);
        }
        (Resource::Unit, Holds::Wrong) => {
            sh("systemctl", &["stop", SERVICE]);
            sh("systemctl", &["disable", SERVICE]);
        }
    }
}

/// The host before any test: the unit file, the repository with both
/// versions of the package, and nothing of the Canon's.
pub fn prepare() {
    std::fs::write(format!("/etc/systemd/system/{SERVICE}"), UNIT_FILE).unwrap();
    assert!(sh("systemctl", &["daemon-reload"]));
    build(PACKAGE, "1.0-1", None);
    build(PACKAGE, "2.0-1", None);
    publish();
}

/// The artifact and its bundle, written once.
pub fn artifact(dir: &Path) -> (PathBuf, PathBuf) {
    let file = dir.join("demonstration.cbor");
    std::fs::write(&file, encode(&demonstration(), Profile::Cbor)).unwrap();
    let bundle = dir.join("bundle");
    std::fs::create_dir_all(&bundle).unwrap();
    let name: String = digest(CONFIG)
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    std::fs::write(bundle.join(name), CONFIG).unwrap();
    (file, bundle)
}
