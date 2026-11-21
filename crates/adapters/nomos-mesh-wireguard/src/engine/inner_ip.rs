//! Inner IPv6 (40B) + UDP (8B) header framing at bytes `[16..64]` of a tunnel slot.

use std::net::Ipv6Addr;

use nomos_mesh::{INNER_IPV6_UDP_LEN, MeshError, MeshResult};

use crate::boundary::SliceWriter;

/// Endpoint `(IPv6, UDP port)` for inner tunnel traffic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InnerEndpoint {
    /// 128-bit IPv6 address.
    pub addr: Ipv6Addr,
    /// UDP port number.
    pub port: u16,
}

/// Decoded 48-byte inner IPv6+UDP header preceding encapsulated QUIC payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InnerIpv6UdpHeader {
    /// Source IPv6 + port.
    pub src: InnerEndpoint,
    /// Destination IPv6 + port.
    pub dst: InnerEndpoint,
    /// QUIC payload byte length (excluding 8-byte UDP header).
    pub payload_len: u16,
    /// Traffic class byte (carries DSCP + ECN bits).
    pub traffic_class: u8,
    /// UDP checksum from RFC 8200 pseudo-header.
    pub udp_checksum: u16,
}

impl InnerIpv6UdpHeader {
    /// Encodes the 48-byte inner IPv6+UDP header directly into `dst[0..48]` without allocating.
    pub fn encode_into(&self, payload: &[u8], dst: &mut [u8]) -> MeshResult<()> {
        if dst.len() < INNER_IPV6_UDP_LEN {
            return Err(MeshError::PoolExhausted);
        }
        let udp_len = (8usize + payload.len()) as u16;
        let checksum = udp_v6_checksum(
            self.src.addr.octets(),
            self.dst.addr.octets(),
            self.src.port,
            self.dst.port,
            payload,
        );

        let mut writer = SliceWriter::new(&mut dst[..INNER_IPV6_UDP_LEN]);
        let ver_tc_fl: u32 = (6u32 << 28) | ((self.traffic_class as u32) << 20);
        writer.write_u32_be(ver_tc_fl)?;
        writer.write_u16_be(udp_len)?;
        writer.write_bytes(&[17u8, 64u8])?; // Next Header = UDP (17), Hop Limit = 64
        writer.write_bytes(&self.src.addr.octets())?;
        writer.write_bytes(&self.dst.addr.octets())?;
        writer.write_u16_be(self.src.port)?;
        writer.write_u16_be(self.dst.port)?;
        writer.write_u16_be(udp_len)?;
        writer.write_u16_be(checksum)?;
        Ok(())
    }

    /// Parses a 48-byte inner IPv6+UDP header from `src` without allocating.
    pub fn decode_from(src: &[u8]) -> MeshResult<Self> {
        if src.len() < INNER_IPV6_UDP_LEN {
            return Err(MeshError::InvalidConfig(
                "truncated inner IPv6+UDP header".into(),
            ));
        }
        let version = src[0] >> 4;
        if version != 6 || src[6] != 17 {
            return Err(MeshError::InvalidConfig(
                "inner packet must be IPv6 UDP".into(),
            ));
        }
        let traffic_class = ((src[0] & 0x0f) << 4) | (src[1] >> 4);
        let payload_len = u16::from_be_bytes([src[4], src[5]]).saturating_sub(8);
        let mut src_bytes = [0u8; 16];
        let mut dst_bytes = [0u8; 16];
        src_bytes.copy_from_slice(&src[8..24]);
        dst_bytes.copy_from_slice(&src[24..40]);
        let src_port = u16::from_be_bytes([src[40], src[41]]);
        let dst_port = u16::from_be_bytes([src[42], src[43]]);
        let udp_len = u16::from_be_bytes([src[44], src[45]]);
        if udp_len < 8 || udp_len.saturating_sub(8) != payload_len {
            return Err(MeshError::InvalidConfig(
                "inconsistent inner IPv6/UDP lengths".into(),
            ));
        }
        let udp_checksum = u16::from_be_bytes([src[46], src[47]]);
        Ok(Self {
            src: InnerEndpoint {
                addr: Ipv6Addr::from(src_bytes),
                port: src_port,
            },
            dst: InnerEndpoint {
                addr: Ipv6Addr::from(dst_bytes),
                port: dst_port,
            },
            payload_len,
            traffic_class,
            udp_checksum,
        })
    }
}

/// Computes RFC 8200 IPv6 UDP checksum over pseudo-header, UDP header, and payload.
pub fn udp_v6_checksum(
    src_ip: [u8; 16],
    dst_ip: [u8; 16],
    src_port: u16,
    dst_port: u16,
    payload: &[u8],
) -> u16 {
    let mut sum: u32 = 0;
    for chunk in src_ip
        .as_chunks::<2>()
        .0
        .iter()
        .chain(dst_ip.as_chunks::<2>().0.iter())
    {
        sum += u16::from_be_bytes(*chunk) as u32;
    }
    let udp_len = (8usize + payload.len()) as u32;
    sum += udp_len >> 16;
    sum += udp_len & 0xffff;
    sum += 17u32; // Next Header = UDP
    sum += src_port as u32;
    sum += dst_port as u32;
    sum += udp_len & 0xffff;

    let (pairs, rem) = payload.as_chunks::<2>();
    for pair in pairs {
        sum += u16::from_be_bytes(*pair) as u32;
    }
    if let [odd] = rem {
        sum += (*odd as u32) << 8;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    let folded = !(sum as u16);
    if folded == 0 { 0xffff } else { folded }
}
