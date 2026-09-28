//! Fixture core crate.
#![no_std]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

/// Adds one.
pub fn succ(n: u32) -> u32 {
    n + 1
}

#[cfg(test)]
mod tests {
    #[test]
    fn succ_adds_one() {
        assert_eq!(super::succ(1), 2);
    }
}
