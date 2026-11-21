//! Noise_IKpsk2 cryptographic primitives, MACs, BLAKE3 KDF, and Type 3 DoS Cookies.

use std::net::Ipv6Addr;

use nomos_mesh::{MeshError, MeshResult};
use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, Nonce, UnboundKey};
use ring::agreement::{EphemeralPrivateKey, UnparsedPublicKey, X25519, agree_ephemeral};
use ring::rand::{SecureRandom, SystemRandom};
use x25519_dalek::{PublicKey as X25519Public, StaticSecret as X25519Static};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Static X25519 identity keypair zeroized automatically on drop.
#[derive(Clone)]
pub struct StaticKeypair {
    secret: [u8; 32],
    public: [u8; 32],
}

impl Drop for StaticKeypair {
    fn drop(&mut self) {
        self.secret.zeroize();
    }
}

impl ZeroizeOnDrop for StaticKeypair {}

impl core::fmt::Debug for StaticKeypair {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StaticKeypair")
            .field("public_prefix", &&self.public[..4])
            .finish_non_exhaustive()
    }
}

impl StaticKeypair {
    /// Generates a fresh X25519 static keypair from the OS CSPRNG.
    pub fn generate() -> MeshResult<Self> {
        let mut secret = [0u8; 32];
        SystemRandom::new().fill(&mut secret).map_err(|_| {
            MeshError::Crypto("CSPRNG failure generating WireGuard static key".into())
        })?;
        Ok(Self::from_secret_bytes(secret))
    }

    /// Constructs a static keypair from raw 32-byte X25519 scalar material.
    pub fn from_secret_bytes(secret: [u8; 32]) -> Self {
        let sk = X25519Static::from(secret);
        let pk = X25519Public::from(&sk);
        Self {
            secret,
            public: *pk.as_bytes(),
        }
    }

    /// 32-byte X25519 public key.
    pub const fn public_bytes(&self) -> [u8; 32] {
        self.public
    }

    pub(crate) fn dh(&self, peer_pub: &[u8; 32]) -> [u8; 32] {
        dh_x25519(&self.secret, peer_pub)
    }
}

/// Wire message type for stateless DoS Cookie Reply (64 bytes).
pub const MSG_TYPE_COOKIE_REPLY: u32 = 3;
/// Wire byte size of a Type 3 Cookie Reply.
pub const COOKIE_REPLY_LEN: usize = 64;

const LABEL_MAC1: &[u8] = b"mac1----";
const LABEL_COOKIE: &[u8] = b"cookie--";
const COOKIE_ROTATION_NS: u64 = 120_000_000_000;

/// Stateless DoS cookie generator rotating its secret every 120 seconds.
#[derive(Debug)]
pub struct CookieGenerator {
    secret: [u8; 32],
    last_rotated_ns: u64,
}

impl Drop for CookieGenerator {
    fn drop(&mut self) {
        self.secret.zeroize();
    }
}

impl CookieGenerator {
    /// Initializes a cookie generator with CSPRNG secret material.
    pub fn new(now_ns: u64) -> MeshResult<Self> {
        let mut secret = [0u8; 32];
        SystemRandom::new()
            .fill(&mut secret)
            .map_err(|_| MeshError::Crypto("CSPRNG failure for cookie secret".into()))?;
        Ok(Self {
            secret,
            last_rotated_ns: now_ns,
        })
    }

    /// Computes a 16-byte stateless DoS cookie bound to `peer_ip`.
    pub fn cookie_for(&mut self, peer_ip: Ipv6Addr, now_ns: u64) -> [u8; 16] {
        if now_ns.saturating_sub(self.last_rotated_ns) >= COOKIE_ROTATION_NS {
            self.secret = *blake3::keyed_hash(&self.secret, &now_ns.to_le_bytes()).as_bytes();
            self.last_rotated_ns = now_ns;
        }
        let digest = blake3::keyed_hash(&self.secret, &peer_ip.octets());
        let mut out = [0u8; 16];
        out.copy_from_slice(&digest.as_bytes()[..16]);
        out
    }
}

/// Decrypts a Type 3 Cookie Reply (64 bytes) to recover the 16-byte DoS cookie.
pub fn consume_cookie_reply(
    reply: &[u8; COOKIE_REPLY_LEN],
    remote_pub: &[u8; 32],
    last_mac1: &[u8; 16],
) -> MeshResult<[u8; 16]> {
    if u32::from_le_bytes([reply[0], reply[1], reply[2], reply[3]]) != MSG_TYPE_COOKIE_REPLY {
        return Err(MeshError::Crypto("invalid Type 3 cookie reply tag".into()));
    }
    let key = cookie_aead_key(remote_pub);
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&reply[8..20]);
    let mut buf = [0u8; 32];
    buf.copy_from_slice(&reply[32..64]);
    let plain = aead_key(&key)?
        .open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(last_mac1),
            &mut buf,
        )
        .map_err(|_| MeshError::Crypto("cookie reply AEAD verification failed".into()))?;
    let mut cookie = [0u8; 16];
    cookie.copy_from_slice(&plain[..16]);
    Ok(cookie)
}

pub(crate) fn build_cookie_reply(
    responder_pub: &[u8; 32],
    receiver_index: u32,
    mac1: &[u8],
    cookie: &[u8; 16],
) -> MeshResult<[u8; COOKIE_REPLY_LEN]> {
    let mut nonce_full = [0u8; 24];
    SystemRandom::new()
        .fill(&mut nonce_full)
        .map_err(|_| MeshError::Crypto("cookie nonce CSPRNG failure".into()))?;
    let key = cookie_aead_key(responder_pub);
    let mut nonce12 = [0u8; 12];
    nonce12.copy_from_slice(&nonce_full[..12]);
    let mut body = [0u8; 32];
    body[..16].copy_from_slice(cookie);
    let tag = aead_key(&key)?
        .seal_in_place_separate_tag(
            Nonce::assume_unique_for_key(nonce12),
            Aad::from(mac1),
            &mut body[..16],
        )
        .map_err(|_| MeshError::Crypto("cookie reply seal failed".into()))?;
    body[16..32].copy_from_slice(tag.as_ref());

    let mut out = [0u8; COOKIE_REPLY_LEN];
    out[0..4].copy_from_slice(&MSG_TYPE_COOKIE_REPLY.to_le_bytes());
    out[4..8].copy_from_slice(&receiver_index.to_le_bytes());
    out[8..32].copy_from_slice(&nonce_full);
    out[32..64].copy_from_slice(&body);
    Ok(out)
}

fn cookie_aead_key(pub_key: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(LABEL_COOKIE);
    h.update(pub_key);
    *h.finalize().as_bytes()
}

pub(crate) fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= core::hint::black_box(x ^ y);
    }
    core::hint::black_box(diff) == 0
}

pub(crate) fn compute_mac1(peer_pub: &[u8; 32], msg_prefix: &[u8]) -> [u8; 16] {
    let mut kh = blake3::Hasher::new();
    kh.update(LABEL_MAC1);
    kh.update(peer_pub);
    let mac_key = *kh.finalize().as_bytes();
    let digest = blake3::keyed_hash(&mac_key, msg_prefix);
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest.as_bytes()[..16]);
    out
}

pub(crate) fn compute_mac2(cookie: &[u8; 16], msg_prefix: &[u8]) -> [u8; 16] {
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(cookie);
    let digest = blake3::keyed_hash(&key, msg_prefix);
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest.as_bytes()[..16]);
    out
}

pub(crate) fn generate_ephemeral_pair() -> MeshResult<([u8; 32], [u8; 32])> {
    let mut sk_bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut sk_bytes)
        .map_err(|_| MeshError::Crypto("ephemeral key CSPRNG failure".into()))?;
    let sk = X25519Static::from(sk_bytes);
    let pk = *X25519Public::from(&sk).as_bytes();
    Ok((sk_bytes, pk))
}

pub(crate) fn dh_x25519(sk_bytes: &[u8; 32], peer_pub: &[u8; 32]) -> [u8; 32] {
    let sk = X25519Static::from(*sk_bytes);
    *sk.diffie_hellman(&X25519Public::from(*peer_pub)).as_bytes()
}

pub(crate) fn dh_ephemeral_ring(peer_pub: &[u8; 32], local_sk: &[u8; 32]) -> MeshResult<[u8; 32]> {
    let probe = EphemeralPrivateKey::generate(&X25519, &SystemRandom::new())
        .map_err(|_| MeshError::Crypto("ring X25519 init failed".into()))?;
    agree_ephemeral(probe, &UnparsedPublicKey::new(&X25519, peer_pub), |_| ())
        .map_err(|_| MeshError::Crypto("peer X25519 public key rejected".into()))?;
    Ok(dh_x25519(local_sk, peer_pub))
}

pub(crate) fn mix_hash(h: &[u8; 32], data: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(h);
    hasher.update(data);
    *hasher.finalize().as_bytes()
}

pub(crate) fn kdf1(key: &[u8; 32], input: &[u8]) -> [u8; 32] {
    let temp = *blake3::keyed_hash(key, input).as_bytes();
    *blake3::keyed_hash(&temp, &[1u8]).as_bytes()
}

pub(crate) fn kdf2(key: &[u8; 32], input: &[u8]) -> ([u8; 32], [u8; 32]) {
    let temp = *blake3::keyed_hash(key, input).as_bytes();
    let t1 = *blake3::keyed_hash(&temp, &[1u8]).as_bytes();
    let mut b2 = [0u8; 33];
    b2[..32].copy_from_slice(&t1);
    b2[32] = 2u8;
    let t2 = *blake3::keyed_hash(&temp, &b2).as_bytes();
    (t1, t2)
}

pub(crate) fn kdf3(key: &[u8; 32], input: &[u8]) -> ([u8; 32], [u8; 32], [u8; 32]) {
    let temp = *blake3::keyed_hash(key, input).as_bytes();
    let t1 = *blake3::keyed_hash(&temp, &[1u8]).as_bytes();
    let mut b2 = [0u8; 33];
    b2[..32].copy_from_slice(&t1);
    b2[32] = 2u8;
    let t2 = *blake3::keyed_hash(&temp, &b2).as_bytes();
    let mut b3 = [0u8; 33];
    b3[..32].copy_from_slice(&t2);
    b3[32] = 3u8;
    let t3 = *blake3::keyed_hash(&temp, &b3).as_bytes();
    (t1, t2, t3)
}

fn aead_key(key: &[u8; 32]) -> MeshResult<LessSafeKey> {
    Ok(LessSafeKey::new(
        UnboundKey::new(&CHACHA20_POLY1305, key)
            .map_err(|_| MeshError::Crypto("invalid handshake AEAD key".into()))?,
    ))
}

pub(crate) fn aead_seal_fixed<const P: usize, const C: usize>(
    key: &[u8; 32],
    plain: &[u8; P],
    aad: &[u8; 32],
) -> MeshResult<[u8; C]> {
    let mut buf = [0u8; C];
    buf[..P].copy_from_slice(plain);
    let tag = aead_key(key)?
        .seal_in_place_separate_tag(
            Nonce::assume_unique_for_key([0u8; 12]),
            Aad::from(aad),
            &mut buf[..P],
        )
        .map_err(|_| MeshError::Crypto("handshake AEAD seal failed".into()))?;
    buf[P..C].copy_from_slice(tag.as_ref());
    Ok(buf)
}

pub(crate) fn aead_open_fixed<const P: usize, const C: usize>(
    key: &[u8; 32],
    ct: &[u8],
    aad: &[u8; 32],
) -> MeshResult<[u8; P]> {
    let mut buf = [0u8; C];
    buf.copy_from_slice(ct);
    aead_key(key)?
        .open_in_place(
            Nonce::assume_unique_for_key([0u8; 12]),
            Aad::from(aad),
            &mut buf,
        )
        .map_err(|_| MeshError::Crypto("handshake AEAD open failed".into()))?;
    let mut out = [0u8; P];
    out.copy_from_slice(&buf[..P]);
    Ok(out)
}
