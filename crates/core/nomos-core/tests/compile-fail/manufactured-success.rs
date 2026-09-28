//! N6: Succeeded needs a `Verified`, and only `verify` builds one. Writing
//! the evidence by hand, to reach Succeeded outside the verification path,
//! does not compile.
use nomos_core::action::{Stage, Verified};
use nomos_core::observation::Instant;
use nomos_core::resource::ResourcePath;

fn main() {
    let _ = Stage::Succeeded {
        changed: true,
        verified: Verified {
            resource: ResourcePath::new("/etc/hosts").unwrap(),
            since: Instant(0),
        },
    };
}
