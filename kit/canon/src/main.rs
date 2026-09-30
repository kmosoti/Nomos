//! The Canon authoring kit. Describe the host below, then run
//!
//! ```sh
//! cargo run -- out
//! ```
//!
//! to write `out/canon.cbor`, the artifact `nomos-cell` reads, and
//! `out/bundle/`, the content every exact file requirement names, one file
//! per SHA-256 digest. Copy both to `/etc/nomos/canon.cbor` and
//! `/etc/nomos/bundle/` on the host. The artifact carries digests, never
//! file content; the bundle carries the content.

use std::path::Path;

use nomos_canon::artifact::{Profile, encode};
use nomos_canon::model::{Canon, CanonBuilder, RelationKind};
use nomos_core::condition::{
    Activity, Content, DirectoryCondition, Enablement, FileCondition, Metadata, UnitCondition,
};
use nomos_core::resource::{AccountName, Digest, Mode};

/// The content of every exact file requirement, from `files/`.
const MOTD: &[u8] = include_bytes!("../files/motd");

fn digest(bytes: &[u8]) -> Digest {
    Digest::from_bytes(nomos_canon::sha256::digest(bytes))
}

fn owned(owner: &str, mode: &str) -> Metadata {
    Metadata {
        owner: Some(AccountName::new(owner).expect("an account name")),
        group: None,
        mode: Some(Mode::from_octal(mode).expect("an octal mode")),
    }
}

/// The host, as it should be. Edit this.
fn canon() -> Canon {
    CanonBuilder::new("example")
        .directory(
            "/etc/example",
            DirectoryCondition::Present {
                metadata: owned("root", "0755"),
            },
        )
        .file(
            "/etc/motd",
            FileCondition::Present {
                content: Content::Exactly(digest(MOTD)),
                metadata: owned("root", "0644"),
            },
        )
        .sysctl("net.ipv4.ip_forward", "0")
        .unit(
            "cron.service",
            UnitCondition {
                activity: Activity::Active,
                enablement: Enablement::Enabled,
            },
        )
        .relate("directory:/etc/example", RelationKind::After, "file:/etc/motd")
        .build()
        .expect("a valid Canon")
}

/// The content the Canon names, for the bundle.
fn contents() -> Vec<&'static [u8]> {
    vec![MOTD]
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "out".into());
    let out = Path::new(&out);
    let bundle = out.join("bundle");
    std::fs::create_dir_all(&bundle).expect("the output directory");
    std::fs::write(out.join("canon.cbor"), encode(&canon(), Profile::Cbor))
        .expect("the artifact");
    for bytes in contents() {
        let name: String = digest(bytes)
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        std::fs::write(bundle.join(name), bytes).expect("a bundle entry");
    }
    println!("{}", out.join("canon.cbor").display());
}
