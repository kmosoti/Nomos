//! `ArcSwap<KeyTable>` hot-reload store and `IPV6_V6ONLY` `quinn-udp` GSO/GRO socket shell.

use std::io::IoSliceMut;
use std::net::{SocketAddr, SocketAddrV6};
use std::sync::Arc;

use arc_swap::ArcSwap;
use nomos_mesh::{MeshError, MeshResult};
use quinn_udp::{RecvMeta, Transmit, UdpSockRef, UdpSocketState};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
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

/// Pure-IPv6 (`IPV6_V6ONLY`) async UDP socket driver backed by `quinn-udp` GSO, GRO, and ECN.
#[derive(Debug)]
pub struct UdpShell {
    socket: UdpSocket,
    state: UdpSocketState,
    max_gso_segments: usize,
    gro_segments: usize,
    v6_only: bool,
}

impl UdpShell {
    /// Binds a pure-IPv6 (`IPV6_V6ONLY = true`) UDP socket with `quinn-udp` GSO/GRO/ECN.
    pub async fn bind_ipv6(addr: SocketAddrV6) -> MeshResult<Self> {
        let sock = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))
            .map_err(|e| MeshError::Transport(format!("create IPv6 UDP socket: {e}")))?;
        sock.set_only_v6(true)
            .map_err(|e| MeshError::Transport(format!("set IPV6_V6ONLY `{addr}`: {e}")))?;
        sock.set_nonblocking(true)
            .map_err(|e| MeshError::Transport(format!("set_nonblocking `{addr}`: {e}")))?;
        sock.bind(&SockAddr::from(addr))
            .map_err(|e| MeshError::Transport(format!("bind IPv6 UDP `{addr}`: {e}")))?;
        let v6_only = sock
            .only_v6()
            .map_err(|e| MeshError::Transport(format!("read IPV6_V6ONLY `{addr}`: {e}")))?;
        let std_sock: std::net::UdpSocket = sock.into();

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
            v6_only,
        })
    }

    /// True when kernel `IPV6_V6ONLY` socket option is verified active.
    pub const fn is_only_v6(&self) -> bool {
        self.v6_only
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
