//! Hybrid post-quantum encryption combining ML-KEM-768 and X25519 (`X-Wing` style).
//!
//! Combines `ml-kem` (`MlKem768`) with `ring::agreement::X25519` (ephemeral sender)
//! and `x25519-dalek` (static recipient) via domain-separated BLAKE3 key derivation,
//! sealing payloads with `ring::aead::CHACHA20_POLY1305` under `#![forbid(unsafe_code)]`.

use ml_kem::array::Array;
use ml_kem::kem::{Decapsulate, DecapsulationKey, Encapsulate, EncapsulationKey};
use ml_kem::{Ciphertext, Encoded, EncodedSizeUser, MlKem768, MlKem768Params};
use nomos_cipher::{CipherError, CipherResult};
use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, Nonce, UnboundKey};
use ring::agreement::{EphemeralPrivateKey, UnparsedPublicKey, X25519, agree_ephemeral};
use ring::rand::{SecureRandom, SystemRandom};
use x25519_dalek::{PublicKey as X25519Public, StaticSecret as X25519Static};
use zeroize::Zeroize;

pub use crate::pq_types::{HybridIdentity, HybridRecipient};
use crate::pq_types::{RingRng, X25519_KEY_LEN};

/// Wire envelope magic identifying a hybrid ML-KEM-768 + X25519 ciphertext.
pub const HYBRID_MAGIC: &[u8; 8] = b"NMHPQ1\0\0";
const COMBINER_CONTEXT: &str = "nomos/xwing-mlkem768-x25519/v1";
const MLKEM768_CT_LEN: usize = 1088;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

/// Returns `true` if `bytes` begins with the hybrid PQ ciphertext header.
pub fn is_hybrid_ciphertext(bytes: &[u8]) -> bool {
    bytes.starts_with(HYBRID_MAGIC)
}

fn combine_xwing_key(
    ss_pq: &[u8; 32],
    ss_x: &[u8; 32],
    ct_x: &[u8; 32],
    pk_x: &[u8; 32],
    ek_pq_hash: &[u8; 32],
) -> [u8; 32] {
    let mut material = [0u8; 160];
    for (slot, part) in material
        .as_chunks_mut::<32>()
        .0
        .iter_mut()
        .zip([ss_pq, ss_x, ct_x, pk_x, ek_pq_hash])
    {
        *slot = *part;
    }
    let key = blake3::derive_key(COMBINER_CONTEXT, &material);
    material.zeroize();
    key
}

fn build_aad(ct_pq: &[u8], ct_x: &[u8; 32], ek_pq_hash: &[u8; 32], pk_x: &[u8; 32]) -> Vec<u8> {
    [HYBRID_MAGIC.as_slice(), ct_pq, ct_x, ek_pq_hash, pk_x].concat()
}

fn open_aead_key(key_bytes: &mut [u8; 32]) -> CipherResult<LessSafeKey> {
    let unbound = UnboundKey::new(&CHACHA20_POLY1305, key_bytes)
        .map_err(|_| CipherError::Crypto("AEAD key init failed".into()))?;
    key_bytes.zeroize();
    Ok(LessSafeKey::new(unbound))
}

/// Encrypts `plaintext` for `recipient` using hybrid `ML-KEM-768 + X25519` + ChaCha20-Poly1305.
pub fn encrypt_hybrid(recipient: &HybridRecipient, plaintext: &[u8]) -> CipherResult<Vec<u8>> {
    let ek_encoded =
        Encoded::<EncapsulationKey<MlKem768Params>>::try_from(recipient.mlkem_ek.as_slice())
            .map_err(|_| CipherError::Crypto("malformed ML-KEM-768 encapsulation key".into()))?;
    let ek = EncapsulationKey::<MlKem768Params>::from_bytes(&ek_encoded);

    let mut rng = RingRng(SystemRandom::new());
    let (ct_pq_arr, ss_pq_arr) = ek
        .encapsulate(&mut rng)
        .map_err(|_| CipherError::Crypto("ML-KEM-768 encapsulation failed".into()))?;
    let mut ss_pq: [u8; 32] = ss_pq_arr
        .as_slice()
        .try_into()
        .map_err(|_| CipherError::Crypto("unexpected ML-KEM shared secret size".into()))?;

    let sys_rng = SystemRandom::new();
    let eph = EphemeralPrivateKey::generate(&X25519, &sys_rng)
        .map_err(|_| CipherError::Crypto("X25519 ephemeral generation failed".into()))?;
    let mut ct_x = [0u8; X25519_KEY_LEN];
    ct_x.copy_from_slice(
        eph.compute_public_key()
            .map_err(|_| CipherError::Crypto("X25519 public key derivation failed".into()))?
            .as_ref(),
    );
    let peer_pub = UnparsedPublicKey::new(&X25519, &recipient.x25519_pk);
    let mut ss_x = agree_ephemeral(eph, &peer_pub, |shared| {
        let mut out = [0u8; 32];
        out.copy_from_slice(shared);
        out
    })
    .map_err(|_| CipherError::Crypto("X25519 key agreement failed".into()))?;

    let ek_pq_hash = *blake3::hash(&recipient.mlkem_ek).as_bytes();
    let mut aead_key = combine_xwing_key(&ss_pq, &ss_x, &ct_x, &recipient.x25519_pk, &ek_pq_hash);
    ss_pq.zeroize();
    ss_x.zeroize();

    let mut nonce_bytes = [0u8; NONCE_LEN];
    sys_rng
        .fill(&mut nonce_bytes)
        .map_err(|_| CipherError::Crypto("nonce generation failed".into()))?;
    let key = open_aead_key(&mut aead_key)?;

    let aad = build_aad(
        ct_pq_arr.as_slice(),
        &ct_x,
        &ek_pq_hash,
        &recipient.x25519_pk,
    );
    let mut in_out = plaintext.to_vec();
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce_bytes),
        Aad::from(aad.as_slice()),
        &mut in_out,
    )
    .map_err(|_| CipherError::Crypto("hybrid AEAD seal failed".into()))?;

    let mut out = Vec::with_capacity(
        HYBRID_MAGIC.len() + MLKEM768_CT_LEN + X25519_KEY_LEN + NONCE_LEN + in_out.len(),
    );
    out.extend_from_slice(HYBRID_MAGIC);
    out.extend_from_slice(ct_pq_arr.as_slice());
    out.extend_from_slice(&ct_x);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&in_out);
    Ok(out)
}

/// Decrypts a hybrid `ML-KEM-768 + X25519` envelope produced by [`encrypt_hybrid`].
pub fn decrypt_hybrid(identity: &HybridIdentity, ciphertext: &[u8]) -> CipherResult<Vec<u8>> {
    let header_len = HYBRID_MAGIC.len() + MLKEM768_CT_LEN + X25519_KEY_LEN + NONCE_LEN;
    if ciphertext.len() < header_len + TAG_LEN || !is_hybrid_ciphertext(ciphertext) {
        return Err(CipherError::Crypto(
            "malformed hybrid ciphertext envelope".into(),
        ));
    }
    let ct_pq_slice = &ciphertext[8..8 + MLKEM768_CT_LEN];
    let ct_x_off = 8 + MLKEM768_CT_LEN;
    let nonce_off = ct_x_off + X25519_KEY_LEN;

    let mut ct_x = [0u8; X25519_KEY_LEN];
    ct_x.copy_from_slice(&ciphertext[ct_x_off..nonce_off]);
    let mut nonce_bytes = [0u8; NONCE_LEN];
    nonce_bytes.copy_from_slice(&ciphertext[nonce_off..header_len]);

    let dk_encoded =
        Encoded::<DecapsulationKey<MlKem768Params>>::try_from(identity.mlkem_dk.as_slice())
            .map_err(|_| CipherError::Crypto("malformed ML-KEM-768 decapsulation key".into()))?;
    let dk = DecapsulationKey::<MlKem768Params>::from_bytes(&dk_encoded);
    let ct_pq = Ciphertext::<MlKem768>::from(
        Array::try_from(ct_pq_slice)
            .map_err(|_| CipherError::Crypto("malformed ML-KEM-768 ciphertext".into()))?,
    );
    let ss_pq_arr = dk
        .decapsulate(&ct_pq)
        .map_err(|_| CipherError::Crypto("ML-KEM-768 decapsulation failed".into()))?;
    let mut ss_pq: [u8; 32] = ss_pq_arr
        .as_slice()
        .try_into()
        .map_err(|_| CipherError::Crypto("unexpected ML-KEM shared secret size".into()))?;

    let static_sk = X25519Static::from(identity.x25519_sk);
    let mut ss_x = *static_sk
        .diffie_hellman(&X25519Public::from(ct_x))
        .as_bytes();
    let pk_x = *X25519Public::from(&static_sk).as_bytes();
    let ek_pq_hash = *blake3::hash(&identity.mlkem_ek).as_bytes();

    let mut aead_key = combine_xwing_key(&ss_pq, &ss_x, &ct_x, &pk_x, &ek_pq_hash);
    ss_pq.zeroize();
    ss_x.zeroize();
    let key = open_aead_key(&mut aead_key)?;

    let aad = build_aad(ct_pq_slice, &ct_x, &ek_pq_hash, &pk_x);
    let mut in_out = ciphertext[header_len..].to_vec();
    let plain_len = key
        .open_in_place(
            Nonce::assume_unique_for_key(nonce_bytes),
            Aad::from(aad.as_slice()),
            &mut in_out,
        )
        .map_err(|_| CipherError::Crypto("hybrid AEAD authentication failed".into()))?
        .len();
    in_out.truncate(plain_len);
    Ok(in_out)
}
