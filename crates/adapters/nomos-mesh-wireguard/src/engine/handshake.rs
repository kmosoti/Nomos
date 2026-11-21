//! Real WireGuard Noise_IKpsk2 handshake state machine (Type 1 Initiation & Type 2 Response).
//!
//! Uses X25519 ephemeral+static Diffie-Hellman (`ring::agreement::X25519` +
//! `x25519-dalek`), BLAKE3 keyed HKDF transcript derivation, and ChaCha20-Poly1305
//! AEAD with zero static-key shortcuts and zero `unsafe` code.

use std::net::Ipv6Addr;

use nomos_mesh::{MeshError, MeshResult};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::boundary::SessionKeys;
pub use crate::engine::handshake_crypto::{
    COOKIE_REPLY_LEN, CookieGenerator, MSG_TYPE_COOKIE_REPLY, StaticKeypair, consume_cookie_reply,
};
use crate::engine::handshake_crypto::{
    aead_open_fixed, aead_seal_fixed, build_cookie_reply, compute_mac1, compute_mac2, ct_eq,
    dh_ephemeral_ring, dh_x25519, generate_ephemeral_pair, kdf1, kdf2, kdf3, mix_hash,
};

/// Wire message type for Noise_IKpsk2 Initiation (148 bytes).
pub const MSG_TYPE_INITIATION: u32 = 1;
/// Wire message type for Noise_IKpsk2 Response (92 bytes).
pub const MSG_TYPE_RESPONSE: u32 = 2;

/// Wire byte size of a Type 1 Handshake Initiation.
pub const INITIATION_LEN: usize = 148;
/// Wire byte size of a Type 2 Handshake Response.
pub const RESPONSE_LEN: usize = 92;

const CONSTRUCTION: &[u8] = b"Noise_IKpsk2_25519_ChaChaPoly_BLAKE3";
const IDENTIFIER: &[u8] = b"NomosWireGuard v1 nomos@wireguard";

/// Ephemeral initiator state retained between sending Type 1 and consuming Type 2.
pub struct InitiatorHandshake {
    ephemeral_sk: [u8; 32],
    chaining_key: [u8; 32],
    hash: [u8; 32],
    sender_index: u32,
    local_static: StaticKeypair,
    last_mac1: [u8; 16],
}

impl Drop for InitiatorHandshake {
    fn drop(&mut self) {
        self.ephemeral_sk.zeroize();
        self.chaining_key.zeroize();
    }
}

impl ZeroizeOnDrop for InitiatorHandshake {}

/// Outcome of processing a Type 1 initiation on the responder.
#[derive(Debug)]
pub enum ResponderOutcome {
    /// Handshake succeeded: transmit `response` and install `keys`.
    Established {
        /// Serialized 92-byte Type 2 Handshake Response.
        response: [u8; RESPONSE_LEN],
        /// Derived transport session keys.
        keys: Box<SessionKeys>,
    },
    /// Responder is under DoS load and requires valid `mac2`: transmit Type 3 Cookie Reply.
    CookieRequired([u8; COOKIE_REPLY_LEN]),
}

/// Builds a Noise_IKpsk2 Type 1 Handshake Initiation (148 bytes).
pub fn initiate_handshake(
    local_static: &StaticKeypair,
    remote_pub: &[u8; 32],
    sender_index: u32,
    timestamp_ns: u64,
    cookie: Option<[u8; 16]>,
) -> MeshResult<(InitiatorHandshake, [u8; INITIATION_LEN])> {
    let mut ck = *blake3::hash(CONSTRUCTION).as_bytes();
    let mut h = mix_hash(&ck, IDENTIFIER);
    h = mix_hash(&h, remote_pub);

    let (eph_sk, eph_pub) = generate_ephemeral_pair()?;
    h = mix_hash(&h, &eph_pub);
    ck = kdf1(&ck, &eph_pub);

    let mut es = dh_x25519(&eph_sk, remote_pub);
    let (next_ck, mut k1) = kdf2(&ck, &es);
    es.zeroize();
    ck = next_ck;
    let enc_static: [u8; 48] = aead_seal_fixed(&k1, &local_static.public_bytes(), &h)?;
    k1.zeroize();
    h = mix_hash(&h, &enc_static);

    let mut ss = local_static.dh(remote_pub);
    let (next_ck, mut k2) = kdf2(&ck, &ss);
    ss.zeroize();
    ck = next_ck;
    let mut ts = [0u8; 12];
    ts[..8].copy_from_slice(&timestamp_ns.to_be_bytes());
    let enc_ts: [u8; 28] = aead_seal_fixed(&k2, &ts, &h)?;
    k2.zeroize();
    h = mix_hash(&h, &enc_ts);

    let mut msg = [0u8; INITIATION_LEN];
    msg[0..4].copy_from_slice(&MSG_TYPE_INITIATION.to_le_bytes());
    msg[4..8].copy_from_slice(&sender_index.to_le_bytes());
    msg[8..40].copy_from_slice(&eph_pub);
    msg[40..88].copy_from_slice(&enc_static);
    msg[88..116].copy_from_slice(&enc_ts);

    let mac1 = compute_mac1(remote_pub, &msg[..116]);
    msg[116..132].copy_from_slice(&mac1);
    let mac2 = cookie.map_or([0u8; 16], |c| compute_mac2(&c, &msg[..132]));
    msg[132..148].copy_from_slice(&mac2);

    Ok((
        InitiatorHandshake {
            ephemeral_sk: eph_sk,
            chaining_key: ck,
            hash: h,
            sender_index,
            local_static: local_static.clone(),
            last_mac1: mac1,
        },
        msg,
    ))
}

/// Processes a Type 1 Initiation on the responder, returning Type 2 + keys or Type 3 Cookie Reply.
#[allow(clippy::too_many_arguments)]
pub fn respond_handshake(
    local_static: &StaticKeypair,
    expected_initiator_pub: &[u8; 32],
    psk: &[u8; 32],
    msg: &[u8; INITIATION_LEN],
    peer_ip: Ipv6Addr,
    responder_index: u32,
    epoch: u64,
    now_ns: u64,
    under_load: bool,
    cookies: &mut CookieGenerator,
    last_seen_timestamp: &mut u64,
) -> MeshResult<ResponderOutcome> {
    if u32::from_le_bytes([msg[0], msg[1], msg[2], msg[3]]) != MSG_TYPE_INITIATION {
        return Err(MeshError::Crypto("invalid Type 1 handshake tag".into()));
    }
    let expected_mac1 = compute_mac1(&local_static.public_bytes(), &msg[..116]);
    if !ct_eq(&expected_mac1, &msg[116..132]) {
        return Err(MeshError::Crypto(
            "handshake initiation mac1 mismatch".into(),
        ));
    }

    let initiator_index = u32::from_le_bytes([msg[4], msg[5], msg[6], msg[7]]);
    if under_load {
        let cookie = cookies.cookie_for(peer_ip, now_ns);
        let expected_mac2 = compute_mac2(&cookie, &msg[..132]);
        if !ct_eq(&expected_mac2, &msg[132..148]) {
            let reply = build_cookie_reply(
                &local_static.public_bytes(),
                initiator_index,
                &msg[116..132],
                &cookie,
            )?;
            return Ok(ResponderOutcome::CookieRequired(reply));
        }
    }

    let mut ck = *blake3::hash(CONSTRUCTION).as_bytes();
    let mut h = mix_hash(&ck, IDENTIFIER);
    h = mix_hash(&h, &local_static.public_bytes());

    let mut e_i = [0u8; 32];
    e_i.copy_from_slice(&msg[8..40]);
    h = mix_hash(&h, &e_i);
    ck = kdf1(&ck, &e_i);

    let mut es = local_static.dh(&e_i);
    let (next_ck, mut k1) = kdf2(&ck, &es);
    es.zeroize();
    ck = next_ck;
    let peer_static: [u8; 32] = aead_open_fixed::<32, 48>(&k1, &msg[40..88], &h)?;
    k1.zeroize();
    if !ct_eq(&peer_static, expected_initiator_pub) {
        return Err(MeshError::Crypto(
            "unknown peer static key in Noise_IKpsk2".into(),
        ));
    }
    h = mix_hash(&h, &msg[40..88]);

    let mut ss = local_static.dh(&peer_static);
    let (next_ck, mut k2) = kdf2(&ck, &ss);
    ss.zeroize();
    ck = next_ck;
    let ts_bytes: [u8; 12] = aead_open_fixed::<12, 28>(&k2, &msg[88..116], &h)?;
    k2.zeroize();
    let ts = u64::from_be_bytes(
        ts_bytes[..8]
            .try_into()
            .map_err(|_| MeshError::Crypto("ts".into()))?,
    );
    if ts <= *last_seen_timestamp {
        return Err(MeshError::ReplayDetected(ts));
    }
    *last_seen_timestamp = ts;
    h = mix_hash(&h, &msg[88..116]);

    let (e_r_sk, e_r_pub) = generate_ephemeral_pair()?;
    h = mix_hash(&h, &e_r_pub);
    ck = kdf1(&ck, &e_r_pub);

    let mut ee = dh_ephemeral_ring(&e_i, &e_r_sk)?;
    ck = kdf1(&ck, &ee);
    ee.zeroize();

    let mut se = dh_x25519(&e_r_sk, &peer_static);
    ck = kdf1(&ck, &se);
    se.zeroize();

    let (next_ck, tau, mut k3) = kdf3(&ck, psk);
    ck = next_ck;
    h = mix_hash(&h, &tau);
    let enc_empty: [u8; 16] = aead_seal_fixed(&k3, &[], &h)?;
    k3.zeroize();

    let (rx_key, tx_key) = kdf2(&ck, &[]);
    ck.zeroize();

    let mut resp = [0u8; RESPONSE_LEN];
    resp[0..4].copy_from_slice(&MSG_TYPE_RESPONSE.to_le_bytes());
    resp[4..8].copy_from_slice(&responder_index.to_le_bytes());
    resp[8..12].copy_from_slice(&initiator_index.to_le_bytes());
    resp[12..44].copy_from_slice(&e_r_pub);
    resp[44..60].copy_from_slice(&enc_empty);
    let resp_mac1 = compute_mac1(&peer_static, &resp[..60]);
    resp[60..76].copy_from_slice(&resp_mac1);

    Ok(ResponderOutcome::Established {
        response: resp,
        keys: Box::new(SessionKeys::new(
            responder_index,
            initiator_index,
            tx_key,
            rx_key,
            epoch,
        )),
    })
}

impl InitiatorHandshake {
    /// Last `mac1` emitted in Type 1, used to decrypt a Type 3 Cookie Reply.
    pub const fn last_mac1(&self) -> [u8; 16] {
        self.last_mac1
    }

    /// Consumes a Type 2 Handshake Response and derives matching [`SessionKeys`].
    pub fn consume_response(
        mut self,
        resp: &[u8; RESPONSE_LEN],
        psk: &[u8; 32],
        epoch: u64,
    ) -> MeshResult<SessionKeys> {
        if u32::from_le_bytes([resp[0], resp[1], resp[2], resp[3]]) != MSG_TYPE_RESPONSE {
            return Err(MeshError::Crypto("invalid Type 2 handshake tag".into()));
        }
        let responder_index = u32::from_le_bytes([resp[4], resp[5], resp[6], resp[7]]);
        let echoed_index = u32::from_le_bytes([resp[8], resp[9], resp[10], resp[11]]);
        if echoed_index != self.sender_index {
            return Err(MeshError::Crypto(
                "handshake response receiver_index mismatch".into(),
            ));
        }
        let expected_mac1 = compute_mac1(&self.local_static.public_bytes(), &resp[..60]);
        if !ct_eq(&expected_mac1, &resp[60..76]) {
            return Err(MeshError::Crypto("handshake response mac1 mismatch".into()));
        }

        let mut e_r = [0u8; 32];
        e_r.copy_from_slice(&resp[12..44]);
        let mut h = mix_hash(&self.hash, &e_r);
        let mut ck = kdf1(&self.chaining_key, &e_r);

        let mut ee = dh_x25519(&self.ephemeral_sk, &e_r);
        ck = kdf1(&ck, &ee);
        ee.zeroize();

        let mut se = self.local_static.dh(&e_r);
        ck = kdf1(&ck, &se);
        se.zeroize();

        let (next_ck, tau, mut k3) = kdf3(&ck, psk);
        ck = next_ck;
        h = mix_hash(&h, &tau);
        let _: [u8; 0] = aead_open_fixed::<0, 16>(&k3, &resp[44..60], &h)?;
        k3.zeroize();

        let (tx_key, rx_key) = kdf2(&ck, &[]);
        ck.zeroize();
        self.chaining_key.zeroize();
        Ok(SessionKeys::new(
            self.sender_index,
            responder_index,
            tx_key,
            rx_key,
            epoch,
        ))
    }
}
