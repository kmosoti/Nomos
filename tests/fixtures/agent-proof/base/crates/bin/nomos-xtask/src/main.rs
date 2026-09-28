//! Fixture verifier.

fn main() {}

/// A gate: rejects an empty name.
fn gate(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("empty");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_empty_name_is_rejected() {
        assert_eq!(super::gate(""), Err("empty"));
    }
}
