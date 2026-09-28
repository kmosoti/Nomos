//! The generator under test: a Canon written in Rust with the typed
//! builder, from its source alone.

use nomos_canon::model::{CanonBuilder, RelationKind, Requirement, ServiceRequirement};
use nomos_core::condition::{Content, FileCondition};
use nomos_core::resource::Digest;

fn main() -> std::process::ExitCode {
    let config = Digest::from_bytes([0xab; 32]);
    let canon = CanonBuilder::new("telemetry-node")
        .resource(
            "/etc/nomos/cell.conf",
            Requirement::File(FileCondition::Present {
                content: Content::Exactly(config),
            }),
            &["file:/etc/nomos/cell.conf"],
            &[],
        )
        .file(
            "/etc/nomos/ca.pem",
            FileCondition::Present {
                content: Content::Any,
            },
        )
        .resource(
            "/run/nomos-cell",
            Requirement::Service(ServiceRequirement::Running),
            &["systemd:nomos-cell.service"],
            &["node-a"],
        )
        .file("/etc/motd", FileCondition::Absent)
        .relate(
            "/etc/nomos/cell.conf",
            RelationKind::OnChange,
            "/run/nomos-cell",
        )
        .relate(
            "/etc/nomos/ca.pem",
            RelationKind::Requires,
            "/run/nomos-cell",
        )
        .relate("/etc/motd", RelationKind::After, "/etc/nomos/ca.pem")
        .build();
    match canon {
        Ok(c) => canon_authoring::emit(&c),
        Err(_) => std::process::ExitCode::FAILURE,
    }
}
