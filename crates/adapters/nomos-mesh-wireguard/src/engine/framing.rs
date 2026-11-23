//! Zero-allocation TX packet framing and RX inner IPv6/UDP validation helpers.

use nomos_mesh::{
    AEAD_TAG_LEN, INNER_IPV6_UDP_LEN, INNER_OFFSET, MeshError, MeshResult, WG_HEADER_LEN,
};

use crate::boundary::{
    BufPool, CipherDirection, CipherTask, CoreOutput, DecryptedRx, EcnCodepoint, FixedRing,
    PacketLayout, SliceWriter,
};
use crate::engine::inner_ip::{
    InnerEndpoint, InnerIpv6UdpHeader, udp_v6_checksum, validate_inner_endpoints,
};
use crate::engine::sanstun::{PeerTunnelState, ipv6_in_prefix};

const WG_TYPE_TRANSPORT: u32 = 4;

pub(crate) fn encode_tx_slot(
    peer: &mut PeerTunnelState,
    outer_pmtu: u16,
    quic_payload: &[u8],
    src: InnerEndpoint,
    dst: InnerEndpoint,
    ecn: EcnCodepoint,
    pool: &mut BufPool,
) -> MeshResult<CipherTask> {
    let layout = PacketLayout::for_quic_payload(quic_payload.len(), outer_pmtu)?;
    if !peer.established {
        return Err(MeshError::Crypto("peer session not established".into()));
    }
    let counter = peer.next_tx_counter();
    let receiver_index = peer.sender_index;
    let handle = pool.acquire()?;
    let slot = pool.slice_mut(handle)?;
    if slot.len() < layout.total_len {
        pool.release(handle);
        return Err(MeshError::PoolExhausted);
    }

    {
        let mut wg = SliceWriter::new(&mut slot[..WG_HEADER_LEN]);
        wg.write_u32_le(WG_TYPE_TRANSPORT)?;
        wg.write_u32_le(receiver_index)?;
        wg.write_u64_le(counter)?;
    }

    let inner_hdr = InnerIpv6UdpHeader {
        src,
        dst,
        payload_len: quic_payload.len() as u16,
        traffic_class: match ecn {
            EcnCodepoint::None => 0,
            EcnCodepoint::Ect1 => 0b01,
            EcnCodepoint::Ect0 => 0b10,
            EcnCodepoint::Ce => 0b11,
        },
        udp_checksum: 0,
    };
    inner_hdr.encode_into(
        quic_payload,
        &mut slot[WG_HEADER_LEN..(WG_HEADER_LEN + INNER_IPV6_UDP_LEN)],
    )?;

    slot[layout.payload_range()].copy_from_slice(quic_payload);
    slot[layout.tag_range()].fill(0);

    let bytes = quic_payload.len() as u64;
    peer.tx_packets = peer.tx_packets.saturating_add(1);
    peer.tx_bytes = peer.tx_bytes.saturating_add(bytes);
    peer.session_bytes = peer.session_bytes.saturating_add(bytes);

    Ok(CipherTask {
        handle,
        peer_slot: peer.peer_slot,
        receiver_index,
        counter,
        wire_len: layout.total_len,
        direction: CipherDirection::Encrypt,
        ecn,
    })
}

pub(crate) fn verify_and_deliver_rx(
    peer: &mut PeerTunnelState,
    outer_pmtu: u16,
    rx: &DecryptedRx,
    pool: &BufPool,
    out: &mut FixedRing<CoreOutput>,
) -> MeshResult<bool> {
    if !peer.established
        || rx.wire_len < INNER_OFFSET + AEAD_TAG_LEN
        || rx.wire_len > outer_pmtu as usize
    {
        return Ok(false);
    }
    if peer.replay.check_and_accept(rx.counter).is_err() {
        return Ok(false);
    }

    let buf = pool.slice(rx.handle, rx.wire_len)?;
    let Ok(hdr) = InnerIpv6UdpHeader::decode_from(&buf[WG_HEADER_LEN..INNER_OFFSET]) else {
        return Ok(false);
    };
    let payload_len = rx.wire_len - INNER_OFFSET - AEAD_TAG_LEN;
    if hdr.payload_len as usize != payload_len {
        return Ok(false);
    }
    if !validate_inner_endpoints(hdr.src, hdr.dst) {
        return Ok(false);
    }
    if let Some((prefix, prefix_len)) = peer.allowed_prefix
        && !ipv6_in_prefix(hdr.src.addr, prefix, prefix_len)
    {
        return Ok(false);
    }
    if let Some(dst_policy) = peer.allowed_dst
        && !dst_policy.allows(hdr.dst)
    {
        return Ok(false);
    }
    let payload = &buf[INNER_OFFSET..(INNER_OFFSET + payload_len)];
    let expected_csum = udp_v6_checksum(
        hdr.src.addr.octets(),
        hdr.dst.addr.octets(),
        hdr.src.port,
        hdr.dst.port,
        payload,
    );
    if hdr.udp_checksum == 0 || hdr.udp_checksum != expected_csum {
        return Ok(false);
    }

    peer.rx_packets = peer.rx_packets.saturating_add(1);
    peer.rx_bytes = peer.rx_bytes.saturating_add(payload_len as u64);
    peer.session_bytes = peer.session_bytes.saturating_add(payload_len as u64);
    out.push(CoreOutput::DeliverInner {
        handle: rx.handle,
        peer_slot: rx.peer_slot,
        payload_offset: INNER_OFFSET,
        payload_len,
        ecn: rx.ecn,
    })?;
    Ok(true)
}
