//! `check-canon-build`: a Canon crate's build runs no code but the
//! toolchain's and the generator's own (canon-ir.md, Provenance).
//!
//! Over `cargo metadata` of the crate's own workspace, resolved from its
//! lockfile, every package reachable from a workspace member by a normal
//! or build dependency is refused when it has a procedural-macro target or
//! a build script. Development dependencies are not part of the build that
//! writes the artifact, and are not followed. The check reads the graph;
//! it builds nothing and runs nothing of the crate's.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use serde_json::Value;

use crate::layers;

/// One package the rule refuses.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Violation {
    /// The stable code: `canon-build-proc-macro` or `canon-build-script`.
    pub(crate) code: &'static str,
    /// The package, as `name version`.
    pub(crate) package: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.package)
    }
}

/// The packages of the Canon crate at `manifest_path` that the rule
/// refuses, in order; empty when the build runs no code of its own.
pub(crate) fn check(manifest_path: &Path) -> Result<Vec<Violation>, String> {
    let meta = layers::metadata(
        &layers::Options {
            manifest_path: manifest_path.to_path_buf(),
            locked: true,
            offline: false,
        },
        &[],
    )?;
    let packages: BTreeMap<&str, &Value> = meta["packages"]
        .as_array()
        .ok_or("metadata has no packages")?
        .iter()
        .filter_map(|p| Some((p["id"].as_str()?, p)))
        .collect();
    let nodes: BTreeMap<&str, &Value> = meta["resolve"]["nodes"]
        .as_array()
        .ok_or("metadata has no resolve graph")?
        .iter()
        .filter_map(|n| Some((n["id"].as_str()?, n)))
        .collect();
    let mut pending: Vec<&str> = meta["workspace_members"]
        .as_array()
        .ok_or("metadata has no workspace members")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let mut seen = BTreeSet::new();
    let mut violations = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        let package = packages
            .get(id)
            .ok_or_else(|| format!("{id} is resolved but not described"))?;
        let name = format!(
            "{} {}",
            package["name"].as_str().unwrap_or("?"),
            package["version"].as_str().unwrap_or("?")
        );
        for target in package["targets"].as_array().into_iter().flatten() {
            for kind in target["kind"].as_array().into_iter().flatten() {
                let code = match kind.as_str() {
                    Some("proc-macro") => "canon-build-proc-macro",
                    Some("custom-build") => "canon-build-script",
                    _ => continue,
                };
                violations.insert(Violation {
                    code,
                    package: name.clone(),
                });
            }
        }
        let node = nodes
            .get(id)
            .ok_or_else(|| format!("{id} has no node in the resolve graph"))?;
        for dep in node["deps"].as_array().into_iter().flatten() {
            let built = dep["dep_kinds"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|k| matches!(k["kind"].as_str(), None | Some("build")));
            if built && let Some(next) = dep["pkg"].as_str() {
                pending.push(next);
            }
        }
    }
    Ok(violations.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }

    fn codes(crate_dir: &str) -> Vec<&'static str> {
        check(&repo().join(crate_dir).join("Cargo.toml"))
            .unwrap()
            .into_iter()
            .map(|v| v.code)
            .collect()
    }

    /// The authoring kit an operator copies builds no code of its own.
    #[test]
    fn the_authoring_kit_passes() {
        assert_eq!(codes("kit/canon"), Vec::<&str>::new());
    }

    /// So does the authoring crate `build-hermeticity` builds.
    #[test]
    fn the_hermeticity_authoring_crate_passes() {
        assert_eq!(codes("tests/fixtures/canon/authoring"), Vec::<&str>::new());
    }

    /// Negative control: a build script, the one `build-hermeticity` sees.
    #[test]
    fn a_build_script_is_refused() {
        assert_eq!(
            codes("tests/fixtures/canon/authoring-build-script"),
            vec!["canon-build-script"]
        );
    }

    /// Negative control: a procedural macro, the one it cannot see.
    #[test]
    fn a_procedural_macro_is_refused() {
        let found = check(
            &repo()
                .join("tests/fixtures/canon/authoring-proc-macro")
                .join("Cargo.toml"),
        )
        .unwrap();
        assert_eq!(
            found,
            vec![Violation {
                code: "canon-build-proc-macro",
                package: "canon-authoring-macro 0.0.0".to_string(),
            }]
        );
    }
}
