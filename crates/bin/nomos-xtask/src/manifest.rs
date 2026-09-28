//! SHA-256 manifest verification, the `sha256sum -c` contract in Rust.

use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::{Code, VerificationError, Verified, require};

/// Returns the hex SHA-256 digest of a file's bytes.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// One manifest line: a file name and whether its digest matched.
pub(crate) type Entry = (String, bool);

/// Verifies every `<hex>  <name>` line of a manifest against the files beside it.
///
/// The manifest is the snapshot's definition of its own contents, so every
/// failure is fatal: an unreadable or empty manifest, a malformed line, a name
/// that leaves the snapshot directory, a name listed twice, a named file that
/// is missing, and any digest that does not match.
pub(crate) fn verify(manifest: &Path) -> Verified<Vec<Entry>> {
    let dir = manifest.parent().unwrap_or_else(|| Path::new("."));
    let text = std::fs::read_to_string(manifest).map_err(|e| {
        VerificationError::new(
            Code::ManifestUnreadable,
            format!("{}: {e}", manifest.display()),
        )
    })?;
    let mut entries = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let at = format!("{}:{}", manifest.display(), index + 1);
        let (expected, name) = line
            .split_once("  ")
            .map(|(h, n)| (h.trim(), n.trim_start_matches('*').trim()))
            .filter(|(h, n)| {
                h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()) && !n.is_empty()
            })
            .ok_or_else(|| {
                VerificationError::new(
                    Code::ManifestMalformed,
                    format!("{at}: not a sha256sum line"),
                )
            })?;
        require(
            !name.contains('/') && !name.contains('\\') && name != "." && name != "..",
            Code::ManifestMalformed,
            || format!("{at}: {name} is not a file in the snapshot directory"),
        )?;
        require(
            seen.insert(name.to_owned()),
            Code::ManifestDuplicateEntry,
            || format!("{at}: {name} is listed more than once"),
        )?;
        let path = dir.join(name);
        let bytes = std::fs::read(&path).map_err(|e| {
            let code = if e.kind() == ErrorKind::NotFound {
                Code::ManifestNamesMissingFile
            } else {
                Code::ManifestUnreadable
            };
            VerificationError::new(code, format!("{}: {e}", path.display()))
        })?;
        entries.push((
            name.to_owned(),
            sha256_hex(&bytes) == expected.to_ascii_lowercase(),
        ));
    }
    require(!entries.is_empty(), Code::ManifestEmpty, || {
        format!("{}: lists no files", manifest.display())
    })?;
    let failed: Vec<&str> = entries
        .iter()
        .filter(|(_, ok)| !ok)
        .map(|(n, _)| n.as_str())
        .collect();
    require(failed.is_empty(), Code::ChecksumMismatch, || {
        format!("checksum mismatch: {}", failed.join(", "))
    })?;
    Ok(entries)
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
