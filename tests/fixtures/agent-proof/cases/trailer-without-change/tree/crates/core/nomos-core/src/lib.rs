//! Fixture core crate.
#![no_std]

/// Adds one.
pub fn succ(n: u32) -> u32 {
    n.wrapping_add(1)
}

#[cfg(test)]
mod tests {
    #[test]
    fn succ_adds_one() {
        assert_eq!(super::succ(1), 2);
    }
}
