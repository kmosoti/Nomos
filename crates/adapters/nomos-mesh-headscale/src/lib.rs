//! Mesh adapter for Headscale ([ADR 0001](../../../docs/adr/0001-headscale-mesh-adapter.md)).
//!
//! Headscale coordination awaits Phase 6 composition; the adapter currently
//! implements [`nomos_mesh::MeshPort`] cleanly by failing closed with
//! [`MeshError::UnsupportedBackend`].

use nomos_core::plan::Generation;
use nomos_core::resource::Digest;
use nomos_mesh::{MeshError, MeshIdentity, MeshPort, MeshPosture, MeshResult, TunnelConfig};

/// Placeholder Headscale mesh adapter wired to [`nomos_mesh`].
#[derive(Clone, Debug, Default)]
pub struct HeadscaleMeshAdapter {
    control_url: Option<String>,
}

impl HeadscaleMeshAdapter {
    /// Creates an adapter targeting an optional Headscale coordination URL.
    pub fn new(control_url: Option<String>) -> Self {
        Self { control_url }
    }

    /// Configured coordination server URL, if any.
    pub fn control_url(&self) -> Option<&str> {
        self.control_url.as_deref()
    }

    fn unsupported(&self) -> MeshError {
        let target = self.control_url.as_deref().unwrap_or("unconfigured");
        MeshError::UnsupportedBackend(format!(
            "Headscale transport (`{target}`) is not yet implemented"
        ))
    }
}

impl MeshPort for HeadscaleMeshAdapter {
    fn identity(&self) -> MeshResult<MeshIdentity> {
        let _zero = Digest::from_bytes([0u8; 32]);
        Err(self.unsupported())
    }

    fn configure_tunnel(&self, config: &TunnelConfig) -> MeshResult<u16> {
        let _mtu = config.validate()?;
        Err(self.unsupported())
    }

    fn posture(&self, _generation: Generation) -> MeshResult<MeshPosture> {
        Err(self.unsupported())
    }
}
