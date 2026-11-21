//! Ed25519 SHA-256 pinned QUIC control-plane adapter for [`nomos_protocol`] ([ADR 0019](../../../docs/adr/0019-wireguard-mesh-and-quic-protocol.md)).

pub mod cert;
pub mod endpoint;
pub mod frame;
pub mod tls;

pub use cert::{PinnedIdentity, parse_sha256_hex, sha256_der_bytes, sha256_der_hex};
pub use endpoint::{ControlHandler, PinnedQuicConnection, QuicControlNode};
pub use frame::{MAX_FRAME_BYTES, read_json_frame, write_json_frame};
pub use tls::{
    PinnedClientVerifier, PinnedServerVerifier, build_client_config, build_server_config,
};
