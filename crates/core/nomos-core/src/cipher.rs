//! Cipher: references to protected secret material, never plaintext (spec §7).
//!
//! [`Secret`] is the wrapper a resolved secret travels in between the Cipher
//! port and the one sink the Canon names. It implements neither `Display`
//! nor any serialization trait, and its `Debug` output is a fixed marker, so
//! the ordinary paths to an Event, a Plan, a log line, or a diagnostic do not
//! compile or do not reveal. That closes the accidental paths, not every
//! path: a caller can still copy the bytes out through [`Secret::expose`],
//! which is why the method is named for what it does (ADR 0013 §3, finding
//! `secrets-scope`).

use core::fmt;

/// Secret material that must not reach a persistent or diagnostic sink (N8).
pub struct Secret<T>(T);

impl<T> Secret<T> {
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

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

#[cfg(test)]
mod tests {
    use alloc::format;

    use super::Secret;

    const SENTINEL: &str = "hunter2-sentinel-0f3a";

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
}
