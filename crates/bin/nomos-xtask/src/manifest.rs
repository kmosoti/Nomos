//! SHA-256 manifest verification, the `sha256sum -c` contract in Rust.

use std::path::Path;

use sha2::{Digest, Sha256};

/// Returns the hex SHA-256 digest of a file's bytes.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// Verifies every `<hex>  <name>` line of a manifest against the files beside it.
///
/// A missing file is an error, not a failed line: the manifest names what the
/// snapshot must contain.
pub(crate) fn verify(manifest: &Path) -> Result<Vec<(String, bool)>, String> {
    let dir = manifest.parent().unwrap_or_else(|| Path::new("."));
    let text =
        std::fs::read_to_string(manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let mut results = Vec::new();
    for (number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let (expected, name) = line
            .split_once("  ")
            .map(|(h, n)| (h.trim(), n.trim_start_matches('*').trim()))
            .filter(|(h, n)| h.len() == 64 && !n.is_empty())
            .ok_or_else(|| {
                format!(
                    "{}:{}: not a sha256sum line",
                    manifest.display(),
                    number + 1
                )
            })?;
        let path = dir.join(name);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        results.push((name.to_owned(), sha256_hex(&bytes) == expected));
    }
    if results.iter().any(|(_, ok)| !ok) {
        let failed: Vec<&str> = results
            .iter()
            .filter(|(_, ok)| !ok)
            .map(|(n, _)| n.as_str())
            .collect();
        return Err(format!("checksum mismatch: {}", failed.join(", ")));
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::sha256_hex;

    #[test]
    fn empty_input_has_the_known_digest() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
