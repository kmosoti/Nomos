//! `ArcSwap<KeyTable>` hot-reload store, `quinn-udp` GSO/GRO socket shell, and [`MeshPort`] adapter.

use std::io::IoSliceMut;
use std::net::{SocketAddr, SocketAddrV6};
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use nomos_core::plan::Generation;
use nomos_mesh::{
    MeshError, MeshIdentity, MeshPort, MeshPosture, MeshResult, PeerTelemetry, TunnelConfig,
};
use quinn_udp::{RecvMeta, Transmit, UdpSockRef, UdpSocketState};
use tokio::net::UdpSocket;

use crate::boundary::{EcnCodepoint, KeyTable, KeyTableUpdate};

/// Lock-free RCU key store shared between control threads and shell AEAD workers.
#[derive(Debug)]
pub struct KeyStore {
    inner: ArcSwap<KeyTable>,
}

impl KeyStore {
    /// Creates an empty key store sized for `max_peers`.
    pub fn with_capacity(max_peers: usize) -> Self {
        Self {
            inner: ArcSwap::from_pointee(KeyTable::with_capacity(max_peers)),
        }
    }

    /// Loads current immutable key table snapshot with wait-free atomicity.
    pub fn load(&self) -> Arc<KeyTable> {
        self.inner.load_full()
    }

    /// Publishes a new key table snapshot incorporating `update`.
    pub fn apply(&self, update: KeyTableUpdate) {
        let next = self.inner.load().apply_update(update);
        self.inner.store(Arc::new(next));
    }
}

/// Pure-IPv6 async UDP socket driver backed by `quinn-udp` GSO, GRO, and ECN metadata.
pub struct UdpShell {
    socket: UdpSocket,
    state: UdpSocketState,
    max_gso_segments: usize,
    gro_segments: usize,
}

impl UdpShell {
    /// Binds a pure-IPv6 UDP socket and configures `quinn-udp` GSO/GRO/ECN offloading.
    pub async fn bind_ipv6(addr: SocketAddrV6) -> MeshResult<Self> {
        let std_sock = std::net::UdpSocket::bind(addr)
            .map_err(|e| MeshError::Transport(format!("bind UDP `{addr}`: {e}")))?;
        std_sock
            .set_nonblocking(true)
            .map_err(|e| MeshError::Transport(format!("set_nonblocking `{addr}`: {e}")))?;
        let state = UdpSocketState::new(UdpSockRef::from(&std_sock))
            .map_err(|e| MeshError::Transport(format!("configure quinn-udp: {e}")))?;
        let max_gso_segments = state.max_gso_segments();
        let gro_segments = state.gro_segments();
        let socket = UdpSocket::from_std(std_sock)
            .map_err(|e| MeshError::Transport(format!("tokio UdpSocket::from_std: {e}")))?;
        Ok(Self {
            socket,
            state,
            max_gso_segments,
            gro_segments,
        })
    }

    /// Local bound IPv6 socket address.
    pub fn local_addr(&self) -> MeshResult<SocketAddr> {
        self.socket
            .local_addr()
            .map_err(|e| MeshError::Transport(format!("local_addr: {e}")))
    }

    /// Maximum GSO segment count supported by kernel socket.
    pub const fn max_gso_segments(&self) -> usize {
        self.max_gso_segments
    }

    /// Kernel GRO segment batch size.
    pub const fn gro_segments(&self) -> usize {
        self.gro_segments
    }

    /// Transmits a packet or GSO segment batch with ECN marking over IPv6.
    pub async fn send_datagram(
        &self,
        destination: SocketAddrV6,
        ecn: EcnCodepoint,
        segment_size: Option<usize>,
        contents: &[u8],
    ) -> MeshResult<()> {
        let transmit = Transmit {
            destination: SocketAddr::V6(destination),
            ecn: ecn.to_udp(),
            contents,
            segment_size,
            src_ip: None,
        };
        loop {
            self.socket
                .writable()
                .await
                .map_err(|e| MeshError::Transport(format!("writable: {e}")))?;
            match self.socket.try_io(tokio::io::Interest::WRITABLE, || {
                self.state.send(UdpSockRef::from(&self.socket), &transmit)
            }) {
                Ok(()) => return Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(e) => return Err(MeshError::Transport(format!("quinn-udp send: {e}"))),
            }
        }
    }

    /// Receives up to `bufs.len()` GRO UDP datagrams with source IPv6 and ECN metadata.
    pub async fn recv_batch(
        &self,
        bufs: &mut [IoSliceMut<'_>],
        metas: &mut [RecvMeta],
    ) -> MeshResult<usize> {
        loop {
            self.socket
                .readable()
                .await
                .map_err(|e| MeshError::Transport(format!("readable: {e}")))?;
            match self.socket.try_io(tokio::io::Interest::READABLE, || {
                self.state.recv(UdpSockRef::from(&self.socket), bufs, metas)
            }) {
                Ok(n) => return Ok(n),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(e) => return Err(MeshError::Transport(format!("quinn-udp recv: {e}"))),
            }
        }
    }
}

/// WireGuard mesh adapter satisfying [`nomos_mesh::MeshPort`].
#[derive(Debug)]
pub struct WireGuardMeshAdapter {
    identity: MeshIdentity,
    config: Mutex<Option<TunnelConfig>>,
    telemetry: Mutex<Vec<PeerTelemetry>>,
}

impl WireGuardMeshAdapter {
    /// Creates an adapter for caller-provided node identity metadata.
    pub fn new(identity: MeshIdentity) -> Self {
        Self {
            identity,
            config: Mutex::new(None),
            telemetry: Mutex::new(Vec::new()),
        }
    }

    /// Records updated peer telemetry counters from runtime engine loops.
    pub fn update_telemetry(&self, peers: Vec<PeerTelemetry>) -> MeshResult<()> {
        let mut guard = self
            .telemetry
            .lock()
            .map_err(|_| MeshError::Transport("telemetry lock poisoned".into()))?;
        *guard = peers;
        Ok(())
    }
}

impl MeshPort for WireGuardMeshAdapter {
    fn identity(&self) -> MeshResult<MeshIdentity> {
        Ok(self.identity.clone())
    }

    fn configure_tunnel(&self, config: &TunnelConfig) -> MeshResult<u16> {
        let mtu = config.validate()?;
        let mut guard = self
            .config
            .lock()
            .map_err(|_| MeshError::Transport("config lock poisoned".into()))?;
        *guard = Some(config.clone());
        Ok(mtu)
    }

    fn posture(&self, generation: Generation) -> MeshResult<MeshPosture> {
        let cfg = self
            .config
            .lock()
            .map_err(|_| MeshError::Transport("config lock poisoned".into()))?
            .clone()
            .ok_or_else(|| MeshError::InvalidConfig("tunnel not configured".into()))?;
        let peers = self
            .telemetry
            .lock()
            .map_err(|_| MeshError::Transport("telemetry lock poisoned".into()))?
            .clone();
        let active_peers = peers.iter().filter(|p| p.last_handshake_ns > 0).count();
        let tunnel_mtu = cfg.validate()?;
        Ok(MeshPosture {
            interface: cfg.interface,
            outer_pmtu: cfg.outer_pmtu,
            tunnel_mtu,
            generation: generation.0,
            active_peers,
            peers,
        })
    }
}
