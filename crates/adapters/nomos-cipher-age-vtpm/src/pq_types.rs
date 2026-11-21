//! Hybrid ML-KEM-768 + X25519 key types and OS CSPRNG adapter.

use core::fmt;
use std::str::FromStr;

use ml_kem::{EncodedSizeUser, KemCore, MlKem768};
use nomos_cipher::{CipherError, CipherResult};
use rand_core::{CryptoRng, Error as RandError, RngCore};
use ring::rand::{SecureRandom, SystemRandom};
use x25519_dalek::{PublicKey as X25519Public, StaticSecret as X25519Static};
use zeroize::Zeroize;

const IDENTITY_PREFIX: &str = "NOMOS-HYBRID-MLKEM768-X25519-V1:";
const RECIPIENT_PREFIX: &str = "nomos1pq1:";

pub(crate) const MLKEM768_EK_LEN: usize = 1184;
pub(crate) const MLKEM768_DK_LEN: usize = 2400;
pub(crate) const X25519_KEY_LEN: usize = 32;

pub(crate) struct RingRng(pub(crate) SystemRandom);

impl RngCore for RingRng {
    fn next_u32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        self.fill_bytes(&mut b);
        u32::from_le_bytes(b)
    }

    fn next_u64(&mut self) -> u64 {
        let mut b = [0u8; 8];
        self.fill_bytes(&mut b);
        u64::from_le_bytes(b)
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        let _ = self.try_fill_bytes(dest);
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), RandError> {
        self.0
            .fill(dest)
            .map_err(|_| RandError::from(core::num::NonZeroU32::MIN))
    }
}

impl CryptoRng for RingRng {}

/// Public hybrid recipient (`ML-KEM-768` encapsulation key + `X25519` public key).
#[derive(Clone, PartialEq, Eq)]
pub struct HybridRecipient {
    pub(crate) mlkem_ek: Vec<u8>,
    pub(crate) x25519_pk: [u8; X25519_KEY_LEN],
}

impl HybridRecipient {
    /// Parses a recipient from raw `(mlkem_ek_1184, x25519_pk_32)` bytes.
    pub fn from_parts(mlkem_ek: Vec<u8>, x25519_pk: [u8; X25519_KEY_LEN]) -> CipherResult<Self> {
        if mlkem_ek.len() != MLKEM768_EK_LEN {
            return Err(CipherError::Crypto(format!(
                "invalid ML-KEM-768 encapsulation key length: {}",
                mlkem_ek.len()
            )));
        }
        Ok(Self {
            mlkem_ek,
            x25519_pk,
        })
    }

    /// Computes a compact BLAKE3 fingerprint of the hybrid recipient.
    pub fn fingerprint_hex(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&self.mlkem_ek);
        hasher.update(&self.x25519_pk);
        hasher.finalize().to_hex().to_string()
    }
}

impl fmt::Display for HybridRecipient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut raw = Vec::with_capacity(MLKEM768_EK_LEN + X25519_KEY_LEN);
        raw.extend_from_slice(&self.mlkem_ek);
        raw.extend_from_slice(&self.x25519_pk);
        write!(f, "{RECIPIENT_PREFIX}{}", hex::encode(&raw))
    }
}

impl fmt::Debug for HybridRecipient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HybridRecipient")
            .field("fingerprint", &self.fingerprint_hex())
            .finish()
    }
}

impl FromStr for HybridRecipient {
    type Err = CipherError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let body = s
            .trim()
            .strip_prefix(RECIPIENT_PREFIX)
            .ok_or_else(|| CipherError::Crypto("missing hybrid recipient prefix".into()))?;
        let raw = hex::decode(body)
            .map_err(|e| CipherError::Crypto(format!("invalid hybrid recipient hex: {e}")))?;
        if raw.len() != MLKEM768_EK_LEN + X25519_KEY_LEN {
            return Err(CipherError::Crypto(format!(
                "invalid hybrid recipient length: {}",
                raw.len()
            )));
        }
        let mut x25519_pk = [0u8; X25519_KEY_LEN];
        x25519_pk.copy_from_slice(&raw[MLKEM768_EK_LEN..]);
        Self::from_parts(raw[..MLKEM768_EK_LEN].to_vec(), x25519_pk)
    }
}

/// Private hybrid identity (`ML-KEM-768` decapsulation key + `X25519` static secret).
pub struct HybridIdentity {
    pub(crate) mlkem_dk: Vec<u8>,
    pub(crate) mlkem_ek: Vec<u8>,
    pub(crate) x25519_sk: [u8; X25519_KEY_LEN],
}

impl Drop for HybridIdentity {
    fn drop(&mut self) {
        self.mlkem_dk.zeroize();
        self.x25519_sk.zeroize();
    }
}

impl fmt::Debug for HybridIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HybridIdentity([redacted])")
    }
}

impl HybridIdentity {
    /// Generates a fresh `ML-KEM-768 + X25519` identity using the OS CSPRNG.
    pub fn generate() -> CipherResult<Self> {
        let mut rng = RingRng(SystemRandom::new());
        let (dk, ek) = MlKem768::generate(&mut rng);
        let mut x25519_sk = [0u8; X25519_KEY_LEN];
        SystemRandom::new()
            .fill(&mut x25519_sk)
            .map_err(|_| CipherError::Crypto("system CSPRNG failure".into()))?;
        Ok(Self {
            mlkem_dk: dk.as_bytes().as_slice().to_vec(),
            mlkem_ek: ek.as_bytes().as_slice().to_vec(),
            x25519_sk,
        })
    }

    /// Derives the corresponding public [`HybridRecipient`].
    pub fn to_public(&self) -> HybridRecipient {
        let pk = X25519Public::from(&X25519Static::from(self.x25519_sk));
        HybridRecipient {
            mlkem_ek: self.mlkem_ek.clone(),
            x25519_pk: *pk.as_bytes(),
        }
    }

    /// Encodes the identity to a single-line secret representation.
    pub fn encode_secret(&self) -> String {
        let mut raw = Vec::with_capacity(MLKEM768_DK_LEN + MLKEM768_EK_LEN + X25519_KEY_LEN);
        raw.extend_from_slice(&self.mlkem_dk);
        raw.extend_from_slice(&self.mlkem_ek);
        raw.extend_from_slice(&self.x25519_sk);
        let encoded = format!("{IDENTITY_PREFIX}{}", hex::encode(&raw));
        raw.zeroize();
        encoded
    }

    /// Parses an identity line produced by [`Self::encode_secret`].
    pub fn parse_secret(line: &str) -> CipherResult<Self> {
        let body = line
            .trim()
            .strip_prefix(IDENTITY_PREFIX)
            .ok_or_else(|| CipherError::Crypto("missing hybrid identity prefix".into()))?;
        let mut raw = hex::decode(body)
            .map_err(|e| CipherError::Crypto(format!("invalid hybrid identity hex: {e}")))?;
        let expected = MLKEM768_DK_LEN + MLKEM768_EK_LEN + X25519_KEY_LEN;
        if raw.len() != expected {
            raw.zeroize();
            return Err(CipherError::Crypto(format!(
                "invalid hybrid identity length: {}",
                raw.len()
            )));
        }
        let mut x25519_sk = [0u8; X25519_KEY_LEN];
        x25519_sk.copy_from_slice(&raw[MLKEM768_DK_LEN + MLKEM768_EK_LEN..]);
        let id = Self {
            mlkem_dk: raw[..MLKEM768_DK_LEN].to_vec(),
            mlkem_ek: raw[MLKEM768_DK_LEN..MLKEM768_DK_LEN + MLKEM768_EK_LEN].to_vec(),
            x25519_sk,
        };
        raw.zeroize();
        Ok(id)
    }
}
