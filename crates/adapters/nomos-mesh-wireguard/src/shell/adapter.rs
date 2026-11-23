//! [`MeshPort`] adapter owning and driving [`CoreEngine`], [`UdpShell`], and [`KeyStore`].

use std::net::{Ipv6Addr, SocketAddrV6};
use std::sync::{Arc, Mutex};

use nomos_core::plan::Generation;
use nomos_mesh::{
    MeshError, MeshIdentity, MeshPort, MeshPosture, MeshResult, PeerTelemetry, TunnelConfig,
};

use crate::boundary::{
    BufPool, CipherTask, CoreOutput, DecryptedRx, EcnCodepoint, FixedRing, KeyTableUpdate,
    SessionKeys,
};
use crate::engine::{CoreEngine, InnerEndpoint};
use crate::shell::crypto::AdaptiveCipher;
use crate::shell::udp_io::{KeyStore, UdpShell};

struct AdapterRuntime {
    config: TunnelConfig,
    engine: CoreEngine,
    pool: BufPool,
    cipher: AdaptiveCipher,
    out_ring: FixedRing<CoreOutput>,
    cipher_ring: FixedRing<CipherTask>,
    key_ring: FixedRing<KeyTableUpdate>,
    tampered_drops: u64,
}

impl core::fmt::Debug for AdapterRuntime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AdapterRuntime")
            .field("interface", &self.config.interface)
            .field("peers", &self.config.peers.len())
            .field("tampered_drops", &self.tampered_drops)
            .finish_non_exhaustive()
    }
}

/// WireGuard mesh adapter satisfying [`nomos_mesh::MeshPort`] by owning and driving
/// the Sans-I/O [`CoreEngine`], lock-free [`KeyStore`], and optional [`UdpShell`].
#[derive(Debug)]
pub struct WireGuardMeshAdapter {
    identity: MeshIdentity,
    key_store: Arc<KeyStore>,
    udp_shell: Mutex<Option<Arc<UdpShell>>>,
    runtime: Mutex<Option<AdapterRuntime>>,
}

impl WireGuardMeshAdapter {
    /// Creates an adapter for caller-provided node identity metadata.
    pub fn new(identity: MeshIdentity) -> Self {
        Self {
            identity,
            key_store: Arc::new(KeyStore::with_capacity(64)),
            udp_shell: Mutex::new(None),
            runtime: Mutex::new(None),
        }
    }

    /// Returns the lock-free RCU [`KeyStore`] owned by this adapter.
    pub fn key_store(&self) -> Arc<KeyStore> {
        Arc::clone(&self.key_store)
    }

    /// Binds and attaches a pure-IPv6 [`UdpShell`] socket to this adapter.
    pub async fn bind_udp_ipv6(&self, bind_addr: SocketAddrV6) -> MeshResult<Arc<UdpShell>> {
        let shell = Arc::new(UdpShell::bind_ipv6(bind_addr).await?);
        let mut guard = self
            .udp_shell
            .lock()
            .map_err(|_| MeshError::Transport("udp_shell lock poisoned".into()))?;
        *guard = Some(Arc::clone(&shell));
        Ok(shell)
    }

    /// Returns the attached [`UdpShell`] if bound.
    pub fn udp_shell(&self) -> MeshResult<Option<Arc<UdpShell>>> {
        self.udp_shell
            .lock()
            .map(|g| g.clone())
            .map_err(|_| MeshError::Transport("udp_shell lock poisoned".into()))
    }

    /// Installs negotiated Noise_IKpsk2 [`SessionKeys`] into both [`CoreEngine`] and [`KeyStore`].
    pub fn install_session(
        &self,
        peer_slot: u16,
        keys: SessionKeys,
        now_ns: u64,
    ) -> MeshResult<()> {
        let mut guard = self
            .runtime
            .lock()
            .map_err(|_| MeshError::Transport("runtime lock poisoned".into()))?;
        let rt = guard
            .as_mut()
            .ok_or_else(|| MeshError::InvalidConfig("tunnel not configured".into()))?;
        rt.engine
            .install_peer_session(peer_slot, keys, now_ns, &mut rt.key_ring)?;
        while let Some(update) = rt.key_ring.pop() {
            self.key_store.apply(update);
        }
        Ok(())
    }

    /// Encapsulates and AEAD-seals an outbound payload for `peer_slot`, returning wire bytes.
    pub fn encrypt_outbound_payload(
        &self,
        peer_slot: u16,
        payload: &[u8],
        src: InnerEndpoint,
        dst: InnerEndpoint,
        ecn: EcnCodepoint,
    ) -> MeshResult<Vec<u8>> {
        let mut guard = self
            .runtime
            .lock()
            .map_err(|_| MeshError::Transport("runtime lock poisoned".into()))?;
        let rt = guard
            .as_mut()
            .ok_or_else(|| MeshError::InvalidConfig("tunnel not configured".into()))?;
        let task =
            rt.engine
                .encapsulate_quic_tx(peer_slot, payload, src, dst, ecn, &mut rt.pool)?;
        let table = self.key_store.load();
        rt.cipher.encrypt_batch(&[task], &mut rt.pool, &table)?;
        let wire = rt.pool.slice(task.handle, task.wire_len)?.to_vec();
        rt.pool.release(task.handle);
        Ok(wire)
    }

    /// Drives one [`CoreEngine::poll`] tick, retiring expired keys from [`KeyStore`] and
    /// encrypting emitted keepalives.
    pub fn poll_once(&self, now_ns: u64, rx_batch: &[DecryptedRx]) -> MeshResult<Vec<CoreOutput>> {
        let mut guard = self
            .runtime
            .lock()
            .map_err(|_| MeshError::Transport("runtime lock poisoned".into()))?;
        let rt = guard
            .as_mut()
            .ok_or_else(|| MeshError::InvalidConfig("tunnel not configured".into()))?;
        rt.engine.poll(
            now_ns,
            rx_batch,
            &mut rt.pool,
            &mut rt.out_ring,
            &mut rt.cipher_ring,
        )?;
        let table = self.key_store.load();
        while let Some(task) = rt.cipher_ring.pop() {
            let _ = rt.cipher.encrypt_batch(&[task], &mut rt.pool, &table);
            rt.pool.release(task.handle);
        }
        let mut outputs = Vec::new();
        while let Some(item) = rt.out_ring.pop() {
            match item {
                CoreOutput::RetirePeer { peer_slot } => {
                    self.key_store.apply(KeyTableUpdate::Retire { peer_slot });
                    outputs.push(item);
                }
                CoreOutput::RecycleHandle { handle } => {
                    rt.pool.release(handle);
                }
                other => outputs.push(other),
            }
        }
        Ok(outputs)
    }
}

impl MeshPort for WireGuardMeshAdapter {
    fn identity(&self) -> MeshResult<MeshIdentity> {
        Ok(self.identity.clone())
    }

    fn configure_tunnel(&self, config: &TunnelConfig) -> MeshResult<u16> {
        let mtu = config.validate()?;
        let local_ip = parse_ipv6_prefix(&config.local_ipv6)?.0;
        let peer_count = config.peers.len().max(1);
        let default_dst = config
            .peers
            .first()
            .and_then(|p| p.allowed_ips_v6.first())
            .and_then(|cidr| parse_ipv6_prefix(cidr).ok())
            .map_or(local_ip, |(ip, _)| ip);

        let mut engine = CoreEngine::new(
            peer_count,
            config.outer_pmtu,
            InnerEndpoint {
                addr: local_ip,
                port: config.listen_port,
            },
            InnerEndpoint {
                addr: default_dst,
                port: config.listen_port,
            },
        )?;
        for (idx, peer) in config.peers.iter().enumerate() {
            if let Some(first_cidr) = peer.allowed_ips_v6.first() {
                let (prefix, len) = parse_ipv6_prefix(first_cidr)?;
                engine.set_peer_allowed_prefix(idx as u16, prefix, len)?;
            }
            engine.set_peer_allowed_destination(idx as u16, local_ip, 128, None)?;
        }

        let rt = AdapterRuntime {
            config: config.clone(),
            engine,
            pool: BufPool::new(128, 8)?,
            cipher: AdaptiveCipher::default(),
            out_ring: FixedRing::with_capacity(128),
            cipher_ring: FixedRing::with_capacity(128),
            key_ring: FixedRing::with_capacity(32),
            tampered_drops: 0,
        };
        let mut guard = self
            .runtime
            .lock()
            .map_err(|_| MeshError::Transport("runtime lock poisoned".into()))?;
        *guard = Some(rt);
        Ok(mtu)
    }

    fn posture(&self, generation: Generation) -> MeshResult<MeshPosture> {
        let guard = self
            .runtime
            .lock()
            .map_err(|_| MeshError::Transport("runtime lock poisoned".into()))?;
        let rt = guard
            .as_ref()
            .ok_or_else(|| MeshError::InvalidConfig("tunnel not configured".into()))?;
        let tunnel_mtu = rt.config.validate()?;

        let peers: Vec<PeerTelemetry> = rt
            .config
            .peers
            .iter()
            .enumerate()
            .map(|(idx, spec)| {
                let st = rt.engine.peers().get(idx);
                PeerTelemetry {
                    name: spec.name.clone(),
                    last_handshake_ns: st.map_or(0, |p| p.last_handshake_ns),
                    rx_bytes: st.map_or(0, |p| p.rx_bytes),
                    tx_bytes: st.map_or(0, |p| p.tx_bytes),
                    rx_packets: st.map_or(0, |p| p.rx_packets),
                    tx_packets: st.map_or(0, |p| p.tx_packets),
                    tampered_drops: rt.tampered_drops,
                }
            })
            .collect();
        let active_peers = rt.engine.peers().iter().filter(|p| p.established).count();

        Ok(MeshPosture {
            interface: rt.config.interface.clone(),
            outer_pmtu: rt.config.outer_pmtu,
            tunnel_mtu,
            generation: generation.0,
            active_peers,
            peers,
        })
    }
}

fn parse_ipv6_prefix(cidr: &str) -> MeshResult<(Ipv6Addr, u8)> {
    let mut parts = cidr.split('/');
    let ip_str = parts
        .next()
        .ok_or_else(|| MeshError::InvalidConfig(format!("empty IPv6 CIDR `{cidr}`")))?;
    let addr = ip_str
        .parse::<Ipv6Addr>()
        .map_err(|e| MeshError::InvalidConfig(format!("invalid IPv6 `{cidr}`: {e}")))?;
    let len = match parts.next() {
        Some(raw) => raw
            .parse::<u8>()
            .map_err(|e| MeshError::InvalidConfig(format!("invalid prefix len `{cidr}`: {e}")))?,
        None => 128,
    };
    if len > 128 {
        return Err(MeshError::InvalidConfig(format!(
            "IPv6 prefix length {len} exceeds 128"
        )));
    }
    Ok((addr, len))
}
