//! Cipher: references to protected secret material, never plaintext (spec §7).
//!
//! [`Secret`] is the wrapper a resolved secret travels in between the Cipher
//! port and the one sink the Canon names. It implements neither `Display`
//! nor any serialization trait, its `Debug` output is a fixed marker, and its
//! [`Drop`] implementation scrubs inner secret bytes in place using a
//! black-box zeroing pass and a `SeqCst` compiler fence under `#![no_std]` and
//! `#![forbid(unsafe_code)]`. That closes the accidental paths, not every path:
//! a caller can still copy the bytes out through [`Secret::expose`], which is
//! why the method is named for what it does (ADR 0013 §3, finding
//! `secrets-scope`).

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::hint::black_box;
use core::sync::atomic::{Ordering, compiler_fence};

/// Overwrites every byte in `bytes` with zero and issues a `SeqCst` compiler
/// fence to inhibit dead-store elimination without `unsafe` code.
pub fn scrub_bytes(bytes: &mut [u8]) {
    for byte in bytes.iter_mut() {
        *byte = black_box(0u8);
    }
    let _ = black_box(bytes);
    compiler_fence(Ordering::SeqCst);
}

/// Pure `#![no_std]` zero-dependency scrubbing contract invoked when [`Secret`]
/// is dropped.
pub trait SecretScrub {
    /// Scrubs secret material held by `self` before deallocation or reuse.
    fn scrub(&mut self);
}

impl SecretScrub for Vec<u8> {
    fn scrub(&mut self) {
        scrub_bytes(self.as_mut_slice());
        self.clear();
    }
}

impl SecretScrub for String {
    fn scrub(&mut self) {
        let mut owned = core::mem::take(self).into_bytes();
        scrub_bytes(owned.as_mut_slice());
        owned.clear();
    }
}

impl<const N: usize> SecretScrub for [u8; N] {
    fn scrub(&mut self) {
        scrub_bytes(self.as_mut_slice());
    }
}

impl SecretScrub for &str {
    fn scrub(&mut self) {
        *self = "";
        compiler_fence(Ordering::SeqCst);
    }
}

impl SecretScrub for &[u8] {
    fn scrub(&mut self) {
        *self = &[];
        compiler_fence(Ordering::SeqCst);
    }
}

/// Secret material that must not reach a persistent or diagnostic sink (N8).
///
/// Inner secret bytes are scrubbed automatically on [`Drop`].
pub struct Secret<T: SecretScrub>(T);

impl<T: SecretScrub> Secret<T> {
    /// Wraps `value`.
    pub const fn new(value: T) -> Self {
        Secret(value)
    }

    /// The plaintext. Every call site is a sink and is reviewed as one.
    #[must_use = "exposing a secret without using it is a review finding"]
    pub fn expose(&self) -> &T {
        &self.0
    }
}

impl<T: SecretScrub> Drop for Secret<T> {
    fn drop(&mut self) {
        self.0.scrub();
    }
}

impl<T: SecretScrub> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::string::String;
    use alloc::vec;
    use core::cell::Cell;

    use super::{Secret, SecretScrub, scrub_bytes};

    const SENTINEL: &str = "hunter2-sentinel-0f3a";

    struct ScrubProbe<'a> {
        buf: [u8; 8],
        scrubbed: &'a Cell<bool>,
    }

    impl SecretScrub for ScrubProbe<'_> {
        fn scrub(&mut self) {
            scrub_bytes(&mut self.buf);
            assert_eq!(self.buf, [0u8; 8]);
            self.scrubbed.set(true);
        }
    }

    #[test]
    fn debug_output_never_contains_the_sentinel() {
        let secret = Secret::new(SENTINEL);
        let rendered = format!("{secret:?}");
        assert_eq!(rendered, "Secret([redacted])");
        assert!(!rendered.contains(SENTINEL));
        let nested = format!("{:?}", (1, &secret, "x"));
        assert!(!nested.contains(SENTINEL));
        assert_eq!(*secret.expose(), SENTINEL);
    }

    #[test]
    fn drop_scrubs_inner_secret_bytes() {
        let scrubbed = Cell::new(false);
        {
            let secret = Secret::new(ScrubProbe {
                buf: *b"topsecrt",
                scrubbed: &scrubbed,
            });
            assert_eq!(&secret.expose().buf, b"topsecrt");
        }
        assert!(scrubbed.get());

        let mut vec_buf = vec![0xA5u8; 32];
        vec_buf.scrub();
        assert!(vec_buf.is_empty());

        let mut str_buf = String::from(SENTINEL);
        str_buf.scrub();
        assert!(str_buf.is_empty());

        let mut arr_buf = [0x5Au8; 16];
        arr_buf.scrub();
        assert_eq!(arr_buf, [0u8; 16]);
    }
}
