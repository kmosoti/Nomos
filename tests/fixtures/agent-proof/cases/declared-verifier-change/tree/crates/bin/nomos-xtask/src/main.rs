//! Fixture verifier.

fn main() {}

/// A gate: rejects an empty or blank name.
fn gate(name: &str) -> Result<(), &'static str> {
    if name.trim().is_empty() {
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
