//! Bounded length-prefixed JSON and canonical CBOR wire framing over QUIC streams.

use nomos_protocol::{ProtocolError, ProtocolResult};
use quinn::{RecvStream, SendStream};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Maximum serialized control frame payload size (1 MiB).
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Wire serialization codec for QUIC control-plane RPC frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WireFormat {
    /// UTF-8 JSON framing.
    #[default]
    Json,
    /// Canonical binary CBOR framing (`ciborium`).
    Cbor,
}

/// Writes a 4-byte big-endian length prefix followed by payload bytes in `format`.
pub async fn write_frame<T: Serialize>(
    send: &mut SendStream,
    format: WireFormat,
    value: &T,
) -> ProtocolResult<()> {
    let payload = match format {
        WireFormat::Json => serde_json::to_vec(value)
            .map_err(|e| ProtocolError::Serialization(format!("encode frame JSON: {e}")))?,
        WireFormat::Cbor => {
            let mut buf = Vec::new();
            ciborium::into_writer(value, &mut buf)
                .map_err(|e| ProtocolError::Serialization(format!("encode frame CBOR: {e}")))?;
            buf
        }
    };
    if payload.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::Framing(format!(
            "outbound frame {} bytes exceeds cap {MAX_FRAME_BYTES}",
            payload.len()
        )));
    }
    let header = (payload.len() as u32).to_be_bytes();
    send.write_all(&header)
        .await
        .map_err(|e| ProtocolError::Transport(format!("write frame header: {e}")))?;
    send.write_all(&payload)
        .await
        .map_err(|e| ProtocolError::Transport(format!("write frame body: {e}")))?;
    Ok(())
}

/// Reads a bounded frame from `recv`, auto-detecting JSON (`'{'`) vs binary CBOR.
pub async fn read_frame_auto<T: DeserializeOwned>(
    recv: &mut RecvStream,
) -> ProtocolResult<(WireFormat, T)> {
    let body = read_raw_frame(recv).await?;
    if body.first() == Some(&b'{') {
        let val = serde_json::from_slice(&body)
            .map_err(|e| ProtocolError::Serialization(format!("decode frame JSON: {e}")))?;
        Ok((WireFormat::Json, val))
    } else {
        let val = ciborium::from_reader(body.as_slice())
            .map_err(|e| ProtocolError::Serialization(format!("decode frame CBOR: {e}")))?;
        Ok((WireFormat::Cbor, val))
    }
}

/// Writes a 4-byte big-endian length-prefixed JSON frame onto `send`.
pub async fn write_json_frame<T: Serialize>(
    send: &mut SendStream,
    value: &T,
) -> ProtocolResult<()> {
    write_frame(send, WireFormat::Json, value).await
}

/// Reads a bounded 4-byte big-endian length-prefixed JSON frame from `recv`.
pub async fn read_json_frame<T: DeserializeOwned>(recv: &mut RecvStream) -> ProtocolResult<T> {
    let body = read_raw_frame(recv).await?;
    serde_json::from_slice(&body)
        .map_err(|e| ProtocolError::Serialization(format!("decode frame JSON: {e}")))
}

/// Writes a 4-byte big-endian length-prefixed canonical CBOR frame onto `send`.
pub async fn write_cbor_frame<T: Serialize>(
    send: &mut SendStream,
    value: &T,
) -> ProtocolResult<()> {
    write_frame(send, WireFormat::Cbor, value).await
}

/// Reads a bounded 4-byte big-endian length-prefixed CBOR frame from `recv`.
pub async fn read_cbor_frame<T: DeserializeOwned>(recv: &mut RecvStream) -> ProtocolResult<T> {
    let body = read_raw_frame(recv).await?;
    ciborium::from_reader(body.as_slice())
        .map_err(|e| ProtocolError::Serialization(format!("decode frame CBOR: {e}")))
}

async fn read_raw_frame(recv: &mut RecvStream) -> ProtocolResult<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf)
        .await
        .map_err(|e| ProtocolError::Framing(format!("read frame header: {e}")))?;
    let frame_len = u32::from_be_bytes(len_buf) as usize;
    if frame_len == 0 || frame_len > MAX_FRAME_BYTES {
        return Err(ProtocolError::Framing(format!(
            "frame length {frame_len} outside 1..={MAX_FRAME_BYTES}"
        )));
    }
    let mut body = vec![0u8; frame_len];
    recv.read_exact(&mut body)
        .await
        .map_err(|e| ProtocolError::Framing(format!("read frame body: {e}")))?;
    Ok(body)
}
