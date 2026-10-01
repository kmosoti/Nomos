//! `check-evidence-tests`: the tests that judge the code cannot be edited as
//! if they were part of it (verification-strategy.md, Test Taxonomy).
//!
//! A test is evidence-bearing when the semantic-mutant corpus names it as the
//! one that must fail against a wrong behavior. The integration tests of a
//! crate are protected by their path (`crates/*/*/tests/` is a verifier
//! path). A test inside a production source file, in a `#[cfg(test)]`
//! module, cannot be protected by a path, so `verification/evidence-oracles.toml`
//! pins it: the SHA-256 of its text, attributes included, with whitespace
//! collapsed so that formatting does not matter. The file is a verifier
//! file, so editing a pinned test is a declared commit with its new pin in it.
//!
//! The check reports, with a stable code:
//!
//! | Code | When |
//! | --- | --- |
//! | `evidence-oracle-changed` | A pinned test's text no longer has its pin |
//! | `evidence-oracle-unpinned` | An inline test the corpus names has no pin |
//! | `evidence-oracle-missing` | A pin names a test that is not defined, or that the corpus no longer names |
//! | `evidence-oracle-not-found` | The corpus names a test defined nowhere |
//! | `evidence-oracle-ambiguous` | The corpus names a test defined more than once |
//! | `evidence-map-stale` | `docs/formal/oracle-map.md` is not what the corpus and the pins generate |
//!
//! A pin guards the text of the test, not what it calls: an edit to the code
//! under test is implementation, and the semantic mutants judge it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The pins, relative to the repository root.
pub(crate) const PINS_PATH: &str = "verification/evidence-oracles.toml";
/// The generated oracle map, relative to the repository root.
pub(crate) const MAP_PATH: &str = "docs/formal/oracle-map.md";
/// The semantic-mutant corpus.
const CORPUS_PATH: &str = "tests/semantic-mutants/corpus.toml";

/// Why the evidence tests are not as they were pinned. The kebab-case form is
/// the stable identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Code {
    Changed,
    Unpinned,
    Missing,
    NotFound,
    Ambiguous,
    MapStale,
}

impl Code {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Code::Changed => "evidence-oracle-changed",
            Code::Unpinned => "evidence-oracle-unpinned",
            Code::Missing => "evidence-oracle-missing",
            Code::NotFound => "evidence-oracle-not-found",
            Code::Ambiguous => "evidence-oracle-ambiguous",
            Code::MapStale => "evidence-map-stale",
        }
    }
}

/// One thing wrong.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Violation {
    pub(crate) code: Code,
    pub(crate) test: String,
    pub(crate) detail: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.code.as_str(), self.test, self.detail)
    }
}

#[derive(Deserialize)]
struct Corpus {
    mutant: Vec<Mutant>,
}

#[derive(Deserialize)]
struct Mutant {
    id: String,
    property: String,
    test: String,
    status: String,
}

#[derive(Deserialize, Default)]
struct Pins {
    #[serde(default)]
    oracle: Vec<Pin>,
}

#[derive(Deserialize)]
struct Pin {
    test: String,
    file: String,
    sha256: String,
}

/// Where a test lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Where {
    /// In an integration test directory: protected by its path.
    Integration,
    /// In a production source file: protected by its pin.
    Inline,
}

/// A test the corpus names, found in the sources.
#[derive(Debug, Clone)]
struct Row {
    test: String,
    properties: BTreeSet<String>,
    mutants: Vec<String>,
    kind: Where,
    file: String,
    sha256: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The SHA-256 of `text` with every run of whitespace collapsed to one space.
fn pin_of(text: &str) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    hex(&Sha256::digest(normalized.as_bytes()))
}

// ---------------------------------------------------------------------------
// Finding a function and its extent

/// If a string, character, or comment starts at `i`, the index after it.
fn skip_literal(src: &[u8], i: usize) -> Option<usize> {
    let at = |k: usize| src.get(k).copied();
    match src[i] {
        b'/' if at(i + 1) == Some(b'/') => Some(
            src[i..]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(src.len(), |n| i + n),
        ),
        b'/' if at(i + 1) == Some(b'*') => {
            let (mut depth, mut k) = (1, i + 2);
            while k < src.len() && depth > 0 {
                if src[k] == b'/' && at(k + 1) == Some(b'*') {
                    depth += 1;
                    k += 2;
                } else if src[k] == b'*' && at(k + 1) == Some(b'/') {
                    depth -= 1;
                    k += 2;
                } else {
                    k += 1;
                }
            }
            Some(k)
        }
        b'"' => {
            let mut k = i + 1;
            while k < src.len() {
                match src[k] {
                    b'\\' => k += 2,
                    b'"' => return Some(k + 1),
                    _ => k += 1,
                }
            }
            Some(src.len())
        }
        b'r' if i == 0 || !(src[i - 1].is_ascii_alphanumeric() || src[i - 1] == b'_') => {
            let mut k = i + 1;
            while at(k) == Some(b'#') {
                k += 1;
            }
            if at(k) != Some(b'"') {
                return None;
            }
            let hashes = k - (i + 1);
            let close: Vec<u8> = std::iter::once(b'"')
                .chain(std::iter::repeat_n(b'#', hashes))
                .collect();
            let from = k + 1;
            let end = src[from..]
                .windows(close.len())
                .position(|w| w == close.as_slice())
                .map_or(src.len(), |n| from + n + close.len());
            Some(end)
        }
        b'\'' => {
            if at(i + 1) == Some(b'\\') {
                // The escaped character is skipped, so that `'\''` closes at
                // its second quote.
                let rest = src.get(i + 3..).unwrap_or_default();
                Some(
                    rest.iter()
                        .position(|&b| b == b'\'')
                        .map_or(src.len(), |n| i + 3 + n + 1),
                )
            } else {
                // A character of one to four bytes closes after it; anything
                // else, such as `'a` and `'static`, is a lifetime.
                let lead = *src.get(i + 1)?;
                let width = match lead {
                    0x00..=0x7f => 1,
                    0xc0..=0xdf => 2,
                    0xe0..=0xef => 3,
                    _ => 4,
                };
                (lead != b'\'' && at(i + 1 + width) == Some(b'\'')).then_some(i + width + 2)
            }
        }
        _ => None,
    }
}

/// The index after the `}` that closes the `{` at `open`.
fn end_of_braces(src: &[u8], open: usize) -> Option<usize> {
    let (mut depth, mut i) = (0usize, open);
    while i < src.len() {
        if let Some(next) = skip_literal(src, i) {
            i = next;
            continue;
        }
        match src[i] {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The text of every definition of `fn name` in `src`: the attribute lines
/// directly above it, and the function through its closing brace.
fn definitions(src: &str, name: &str) -> Vec<String> {
    let bytes = src.as_bytes();
    let needle = format!("fn {name}");
    let mut out = Vec::new();
    for (at, _) in src.match_indices(&needle) {
        let before_ok =
            at == 0 || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_');
        let after = bytes.get(at + needle.len()).copied();
        let after_ok = matches!(after, Some(b'(' | b'<' | b' '));
        if !(before_ok && after_ok) {
            continue;
        }
        // Skip a match inside a comment or a string: the line must hold code.
        let line_start = src[..at].rfind('\n').map_or(0, |n| n + 1);
        if src[line_start..at].trim_start().starts_with("//") {
            continue;
        }
        let Some(open) =
            (at..bytes.len()).find(|&i| skip_literal(bytes, i).is_none() && bytes[i] == b'{')
        else {
            continue;
        };
        let Some(end) = end_of_braces(bytes, open) else {
            continue;
        };
        // The attribute lines directly above, comments between them skipped.
        let mut start = line_start;
        let mut probe = line_start;
        while probe > 0 {
            let line_end = probe - 1;
            let begin = src[..line_end].rfind('\n').map_or(0, |n| n + 1);
            let line = src[begin..line_end].trim();
            if line.starts_with("#[") {
                start = begin;
            } else if !line.starts_with("//") {
                break;
            }
            probe = begin;
        }
        out.push(src[start..end].to_owned());
    }
    out
}

// ---------------------------------------------------------------------------
// The survey

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            walk(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// Every Rust source file under `crates/`, by path relative to `root`.
fn sources(root: &Path) -> Result<Vec<(String, String)>, String> {
    let mut paths = Vec::new();
    let crates = root.join("crates");
    if crates.is_dir() {
        walk(&crates, &mut paths)?;
    }
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let rel = p
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            Ok((rel, text))
        })
        .collect()
}

/// The tests the corpus names, found in the sources, and what is wrong with
/// finding them.
fn survey(root: &Path) -> Result<(Vec<Row>, Vec<Violation>), String> {
    let corpus_path = root.join(CORPUS_PATH);
    let corpus: Corpus = toml::from_str(
        &std::fs::read_to_string(&corpus_path)
            .map_err(|e| format!("{}: {e}", corpus_path.display()))?,
    )
    .map_err(|e| format!("{}: {}", corpus_path.display(), e.message()))?;
    let files = sources(root)?;
    let mut by_test: BTreeMap<String, (BTreeSet<String>, Vec<String>)> = BTreeMap::new();
    for m in corpus.mutant.iter().filter(|m| m.status == "active") {
        let entry = by_test.entry(m.test.clone()).or_default();
        entry.0.insert(m.property.clone());
        entry.1.push(m.id.clone());
    }
    let (mut rows, mut violations) = (Vec::new(), Vec::new());
    for (test, (properties, mutants)) in by_test {
        let name = test.rsplit("::").next().unwrap_or(&test);
        let mut found: Vec<(Where, String, String)> = Vec::new();
        for (rel, text) in &files {
            for def in definitions(text, name) {
                let kind = if crate::trust::matches_path("crates/*/*/tests/", rel) {
                    Where::Integration
                } else {
                    Where::Inline
                };
                found.push((kind, rel.clone(), def));
            }
        }
        match found.len() {
            0 => violations.push(Violation {
                code: Code::NotFound,
                test: test.clone(),
                detail: format!(
                    "{} names it, and no function of that name is defined under crates/",
                    mutants.join(", ")
                ),
            }),
            1 => {
                let (kind, file, text) = found.remove(0);
                rows.push(Row {
                    test,
                    properties,
                    mutants,
                    kind,
                    file,
                    sha256: pin_of(&text),
                });
            }
            n => violations.push(Violation {
                code: Code::Ambiguous,
                test: test.clone(),
                detail: format!(
                    "defined {n} times ({}): the pin cannot say which the corpus means",
                    found
                        .iter()
                        .map(|(_, f, _)| f.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }),
        }
    }
    Ok((rows, violations))
}

// ---------------------------------------------------------------------------
// The files this command writes

fn render_pins(rows: &[Row]) -> String {
    let mut out = String::from(
        "# The pins of the evidence-bearing tests that live inside production source\n\
         # files (verification-strategy.md, Test Taxonomy). Written by\n\
         # `cargo xtask evidence-tests write`; checked by `check-evidence-tests`.\n\
         # A pin is the SHA-256 of the test's text, attributes included, with\n\
         # whitespace collapsed. This file is a verifier file: editing a pinned\n\
         # test is a commit that declares `Trust-Boundary: verifier` and carries\n\
         # its new pin.\n",
    );
    for row in rows.iter().filter(|r| r.kind == Where::Inline) {
        out.push_str(&format!(
            "\n[[oracle]]\ntest = \"{}\"\nfile = \"{}\"\nsha256 = \"{}\"\n",
            row.test, row.file, row.sha256
        ));
    }
    out
}

fn render_map(rows: &[Row]) -> String {
    let mut out = String::from(
        "# Oracle Map\n\n\
         Generated by `cargo xtask evidence-tests write`, and checked by `cargo xtask check-evidence-tests`, which fails when this file differs from what the semantic-mutant corpus and the pins generate. Do not edit it by hand.\n\n\
         Each row is a test the [semantic-mutant corpus](../../tests/semantic-mutants/README.md) names as the one that must fail against a wrong behavior, with the invariant or property that behavior threatens and what protects the test itself ([Test Taxonomy](verification-strategy.md#test-taxonomy)). An integration test is protected by its path, a verifier path. An inline test is protected by its pin in `verification/evidence-oracles.toml`.\n\n\
         | Test | Invariant or property | Mutants | Defined in | Protected by |\n\
         | --- | --- | --- | --- | --- |\n",
    );
    for row in rows {
        let protection = match row.kind {
            Where::Integration => "its path",
            Where::Inline => "its pin",
        };
        out.push_str(&format!(
            "| `{}` | {} | {} | `{}` | {} |\n",
            row.test,
            row.properties
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
            row.mutants.join(", "),
            row.file,
            protection
        ));
    }
    out
}

/// Writes the pins and the oracle map the corpus and the sources generate.
pub(crate) fn write(root: &Path) -> Result<(), String> {
    let (rows, violations) = survey(root)?;
    if let Some(v) = violations.first() {
        return Err(v.to_string());
    }
    for (path, text) in [
        (PINS_PATH, render_pins(&rows)),
        (MAP_PATH, render_map(&rows)),
    ] {
        let full = root.join(path);
        std::fs::write(&full, text).map_err(|e| format!("{}: {e}", full.display()))?;
    }
    Ok(())
}

/// Checks the pins and the oracle map against the sources.
pub(crate) fn check(root: &Path) -> Result<Vec<Violation>, String> {
    let (rows, mut violations) = survey(root)?;
    let pins_path = root.join(PINS_PATH);
    let pins: Pins = toml::from_str(
        &std::fs::read_to_string(&pins_path)
            .map_err(|e| format!("{}: {e}", pins_path.display()))?,
    )
    .map_err(|e| format!("{}: {}", pins_path.display(), e.message()))?;
    let pinned: BTreeMap<&str, &Pin> = pins.oracle.iter().map(|p| (p.test.as_str(), p)).collect();
    for row in rows.iter().filter(|r| r.kind == Where::Inline) {
        match pinned.get(row.test.as_str()) {
            None => violations.push(Violation {
                code: Code::Unpinned,
                test: row.test.clone(),
                detail: format!(
                    "{} names it, it lives in {}, and {PINS_PATH} has no pin for it",
                    row.mutants.join(", "),
                    row.file
                ),
            }),
            Some(pin) if pin.sha256 != row.sha256 || pin.file != row.file => {
                violations.push(Violation {
                    code: Code::Changed,
                    test: row.test.clone(),
                    detail: format!(
                        "in {}: pinned {} in {}, now {}; an evidence test is edited in a commit that declares `Trust-Boundary: verifier` and carries the new pin",
                        row.file, pin.sha256, pin.file, row.sha256
                    ),
                });
            }
            Some(_) => {}
        }
    }
    let live: BTreeSet<&str> = rows
        .iter()
        .filter(|r| r.kind == Where::Inline)
        .map(|r| r.test.as_str())
        .collect();
    for pin in &pins.oracle {
        if !live.contains(pin.test.as_str()) {
            violations.push(Violation {
                code: Code::Missing,
                test: pin.test.clone(),
                detail: format!(
                    "pinned in {}, and the corpus no longer names an inline test of that name there",
                    pin.file
                ),
            });
        }
    }
    let map_path = root.join(MAP_PATH);
    let map = std::fs::read_to_string(&map_path).unwrap_or_default();
    if map != render_map(&rows) {
        violations.push(Violation {
            code: Code::MapStale,
            test: MAP_PATH.to_owned(),
            detail: "differs from the map the corpus and the sources generate; run `cargo xtask evidence-tests write` and commit it as a specification change".to_owned(),
        });
    }
    violations.sort();
    violations.dedup();
    Ok(violations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    const LIB: &str = "pub fn f() -> u32 { 1 }\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn it_is_one() {\n        assert_eq!(f(), 1);\n    }\n\n    #[test]\n    fn a_regression_test() {\n        assert_eq!(f(), 1);\n    }\n}\n";

    /// A repository with one crate, a corpus naming `it_is_one` (inline) and
    /// `it_works` (an integration test), pins, and the map.
    fn repo(label: &str) -> PathBuf {
        let root = crate::scratch::dir("evidence", label);
        fs::create_dir_all(root.join("crates/core/x/src")).unwrap();
        fs::create_dir_all(root.join("crates/core/x/tests")).unwrap();
        fs::create_dir_all(root.join("tests/semantic-mutants")).unwrap();
        fs::create_dir_all(root.join("verification")).unwrap();
        fs::create_dir_all(root.join("docs/formal")).unwrap();
        fs::write(root.join("crates/core/x/src/lib.rs"), LIB).unwrap();
        fs::write(
            root.join("crates/core/x/tests/it.rs"),
            "#[test]\nfn it_works() {\n    assert!(true);\n}\n",
        )
        .unwrap();
        fs::write(
            root.join(CORPUS_PATH),
            "[[mutant]]\nid = \"SM-X-001\"\nproperty = \"N1\"\ntest = \"tests::it_is_one\"\nstatus = \"active\"\n\n[[mutant]]\nid = \"SM-X-002\"\nproperty = \"N2\"\ntest = \"it_works\"\nstatus = \"active\"\n\n[[mutant]]\nid = \"SM-X-003\"\nproperty = \"N3\"\ntest = \"tests::planned\"\nstatus = \"planned\"\n",
        )
        .unwrap();
        write(&root).unwrap();
        root
    }

    fn codes(root: &Path) -> Vec<&'static str> {
        check(root)
            .unwrap()
            .iter()
            .map(|v| v.code.as_str())
            .collect()
    }

    fn lib(root: &Path, text: &str) {
        fs::write(root.join("crates/core/x/src/lib.rs"), text).unwrap();
    }

    #[test]
    fn pinned_tests_and_a_current_map_pass() {
        let root = repo("ok");
        assert_eq!(codes(&root), Vec::<&str>::new());
        let pins = fs::read_to_string(root.join(PINS_PATH)).unwrap();
        assert!(pins.contains("tests::it_is_one"));
        assert!(
            !pins.contains("it_works"),
            "an integration test needs no pin"
        );
        assert!(
            !pins.contains("regression"),
            "a regression test is not pinned"
        );
    }

    /// Negative control: an inline evidence test is weakened.
    #[test]
    fn an_edited_inline_evidence_test_is_rejected() {
        let root = repo("edited");
        lib(
            &root,
            &LIB.replace(
                "assert_eq!(f(), 1);\n    }\n\n    #[test]\n    fn a_regression",
                "assert!(true);\n    }\n\n    #[test]\n    fn a_regression",
            ),
        );
        assert_eq!(codes(&root), vec!["evidence-oracle-changed"]);
    }

    /// Negative control: the test is silently disabled by removing its
    /// attribute, which leaves the body untouched.
    #[test]
    fn a_test_whose_attribute_is_removed_is_rejected() {
        let root = repo("attribute");
        lib(
            &root,
            &LIB.replace("    #[test]\n    fn it_is_one", "    fn it_is_one"),
        );
        assert_eq!(codes(&root), vec!["evidence-oracle-changed"]);
    }

    #[test]
    fn reformatting_and_unrelated_edits_are_not_changes() {
        let root = repo("format");
        lib(
            &root,
            &LIB.replace(
                "    fn it_is_one() {\n        assert_eq!(f(), 1);\n    }",
                "    fn it_is_one()\n    {\n        assert_eq!(f(), 1);\n    }",
            )
            .replace(
                "pub fn f() -> u32 { 1 }",
                "pub fn f() -> u32 {\n    1\n}\n\n/// New.\npub fn g() {}",
            ),
        );
        assert_eq!(codes(&root), Vec::<&str>::new());
        // A regression test beside it is the generator's to change.
        lib(
            &root,
            &LIB.replace(
                "fn a_regression_test() {\n        assert_eq!(f(), 1);",
                "fn a_regression_test() {\n        assert_eq!(f(), 2);",
            ),
        );
        assert_eq!(codes(&root), Vec::<&str>::new());
    }

    /// Negative control: a new inline test the corpus names, with no pin.
    #[test]
    fn an_inline_test_the_corpus_names_without_a_pin_is_rejected() {
        let root = repo("unpinned");
        fs::write(root.join(PINS_PATH), "").unwrap();
        assert_eq!(codes(&root), vec!["evidence-oracle-unpinned"]);
    }

    /// Negative control: the test is deleted, leaving its pin and its mutant.
    #[test]
    fn a_deleted_inline_test_is_rejected_twice_over() {
        let root = repo("deleted");
        lib(&root, "pub fn f() -> u32 { 1 }\n");
        let found = codes(&root);
        assert!(found.contains(&"evidence-oracle-not-found"), "{found:?}");
        assert!(found.contains(&"evidence-oracle-missing"), "{found:?}");
    }

    #[test]
    fn a_test_defined_twice_is_ambiguous() {
        let root = repo("ambiguous");
        fs::write(
            root.join("crates/core/x/src/other.rs"),
            "#[test]\nfn it_is_one() {}\n",
        )
        .unwrap();
        let found = codes(&root);
        assert!(found.contains(&"evidence-oracle-ambiguous"), "{found:?}");
    }

    #[test]
    fn an_integration_test_is_protected_by_its_path_not_a_pin() {
        let root = repo("integration");
        fs::write(
            root.join("crates/core/x/tests/it.rs"),
            "#[test]\nfn it_works() {\n    assert!(false);\n}\n",
        )
        .unwrap();
        assert_eq!(codes(&root), Vec::<&str>::new());
    }

    #[test]
    fn a_stale_map_is_rejected() {
        let root = repo("map");
        let mut map = fs::read_to_string(root.join(MAP_PATH)).unwrap();
        map.push_str("| `invented` | N1 | SM-X-9 | `x.rs` | its pin |\n");
        fs::write(root.join(MAP_PATH), map).unwrap();
        assert_eq!(codes(&root), vec!["evidence-map-stale"]);
        fs::remove_file(root.join(MAP_PATH)).unwrap();
        assert_eq!(codes(&root), vec!["evidence-map-stale"]);
    }

    /// Braces in strings, characters, raw strings, and comments do not end a
    /// function early, or the pin would cover half of it.
    #[test]
    fn a_function_is_read_through_its_literals() {
        let src = "#[test]\n// }\nfn tricky() {\n    let a = \"}\";\n    let b = '}';\n    let c = r#\"} {\"#;\n    let d = '\\'';\n    /* } /* } */ } */\n    let e: &'static str = \"{\";\n    if a.len() > 0 { let _ = (b, c, d, e); }\n}\nfn after() {}\n";
        let found = definitions(src, "tricky");
        assert_eq!(found.len(), 1);
        assert!(found[0].starts_with("#[test]"), "{}", found[0]);
        assert!(found[0].trim_end().ends_with("}"));
        assert!(!found[0].contains("fn after"), "{}", found[0]);
        assert!(found[0].contains("let _ = (b, c, d, e);"), "{}", found[0]);
    }

    /// The repository's own pins are current and its map is generated, so a
    /// commit that edits an inline evidence test without its pin fails here
    /// as well as in CI.
    #[test]
    fn the_repository_evidence_tests_are_pinned_and_the_map_is_current() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let found = check(&root).unwrap();
        assert!(
            found.is_empty(),
            "{}",
            found
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
