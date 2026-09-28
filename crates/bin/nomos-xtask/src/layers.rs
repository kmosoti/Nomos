//! The dependency-policy check for the hexagonal layout (ADR 0000).
//!
//! Cargo checks that the edges which exist resolve. It does not check that an
//! edge is allowed. This module reads `cargo metadata` twice: the **declared**
//! graph (`--no-deps`), which lists every dependency a manifest names, whether
//! optional, feature-gated, target-specific, dev, or build, and the
//! **resolved** graph under each supported configuration, which lists the
//! edges real builds see. A forbidden edge hidden behind an inactive feature
//! is caught by the first; a forbidden path that only appears once features
//! and targets are applied is caught by the second.
//!
//! A package's layer is its directory under `crates/`. The rule is the table
//! in ADR 0000: core depends on core, ports on core, app on core and ports,
//! adapters on core and exactly one port, and bin on anything; nothing depends
//! on a bin crate, and `nomos-core` depends on no workspace crate.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use serde_json::Value;

/// Targets a real build may use. The resolved graph is checked for each.
const PLATFORMS: [&str; 2] = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"];

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
enum Layer {
    Core,
    Ports,
    App,
    Adapters,
    Bin,
}

impl Layer {
    fn from_dir(dir: &str) -> Option<Layer> {
        match dir {
            "core" => Some(Layer::Core),
            "ports" => Some(Layer::Ports),
            "app" => Some(Layer::App),
            "adapters" => Some(Layer::Adapters),
            "bin" => Some(Layer::Bin),
            _ => None,
        }
    }
}

/// Why an edge is forbidden, if it is.
fn edge_reason(from: Layer, to: Layer) -> Option<&'static str> {
    if to == Layer::Bin {
        return Some("nothing depends on a bin crate");
    }
    match from {
        Layer::Core if to != Layer::Core => Some("core depends only on core"),
        Layer::Ports if to != Layer::Core => Some("ports depend only on core"),
        Layer::App if !matches!(to, Layer::Core | Layer::Ports) => {
            Some("app depends on core and ports, never adapters")
        }
        Layer::Adapters if !matches!(to, Layer::Core | Layer::Ports) => {
            Some("adapters depend on core and one port")
        }
        _ => None,
    }
}

/// One forbidden edge, with everything needed to find it in a manifest.
#[derive(Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Violation {
    graph: String,
    from: String,
    to: String,
    kind: String,
    optional: bool,
    target: Option<String>,
    rename: Option<String>,
    reason: String,
}

impl Violation {
    /// Which graph the edge was found in.
    #[cfg(test)]
    pub(crate) fn graph(&self) -> &str {
        &self.graph
    }

    /// The dependent package.
    #[cfg(test)]
    pub(crate) fn from(&self) -> &str {
        &self.from
    }

    /// The dependency.
    #[cfg(test)]
    pub(crate) fn to(&self) -> &str {
        &self.to
    }

    /// `normal`, `dev`, or `build`.
    #[cfg(test)]
    pub(crate) fn kind(&self) -> &str {
        &self.kind
    }
}

/// The check's report.
#[derive(Serialize)]
pub(crate) struct Report {
    manifest_path: PathBuf,
    packages: usize,
    graphs_checked: Vec<String>,
    violations: Vec<Violation>,
}

impl Report {
    /// The forbidden edges found, sorted and de-duplicated.
    pub(crate) fn violations(&self) -> &[Violation] {
        &self.violations
    }
}

/// How to invoke Cargo.
pub(crate) struct Options {
    /// The workspace root manifest.
    pub(crate) manifest_path: PathBuf,
    /// Pass `--locked`, for the real workspace.
    pub(crate) locked: bool,
    /// Pass `--offline`, for fixtures with no external dependencies.
    pub(crate) offline: bool,
}

struct Member {
    name: String,
    layer: Layer,
}

fn metadata(opts: &Options, extra: &[&str]) -> Result<Value, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(&opts.manifest_path);
    if opts.locked {
        cmd.arg("--locked");
    }
    if opts.offline {
        cmd.arg("--offline");
    }
    cmd.args(extra);
    let out = cmd.output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata {} failed: {}",
            extra.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata output: {e}"))
}

fn members(meta: &Value) -> Result<BTreeMap<String, Member>, String> {
    let root = Path::new(
        meta["workspace_root"]
            .as_str()
            .ok_or("metadata has no workspace_root")?,
    );
    let ids: BTreeSet<&str> = meta["workspace_members"]
        .as_array()
        .ok_or("metadata has no workspace_members")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let mut out = BTreeMap::new();
    for pkg in meta["packages"]
        .as_array()
        .ok_or("metadata has no packages")?
    {
        let id = pkg["id"].as_str().unwrap_or("");
        if !ids.contains(id) {
            continue;
        }
        let name = pkg["name"].as_str().unwrap_or("").to_owned();
        let manifest = Path::new(pkg["manifest_path"].as_str().unwrap_or(""));
        let relative = manifest
            .strip_prefix(root)
            .map_err(|_| format!("{name}: manifest outside the workspace root"))?;
        let mut parts = relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned());
        let layer = match (parts.next().as_deref(), parts.next()) {
            (Some("crates"), Some(dir)) => Layer::from_dir(&dir)
                .ok_or_else(|| format!("{name}: unknown layer directory crates/{dir}"))?,
            _ => {
                return Err(format!(
                    "{name}: not under crates/<layer>/; every workspace crate belongs to one layer"
                ));
            }
        };
        out.insert(id.to_owned(), Member { name, layer });
    }
    Ok(out)
}

fn kind_name(kind: &Value) -> String {
    kind.as_str().unwrap_or("normal").to_owned()
}

/// Runs the check.
pub(crate) fn check(opts: &Options) -> Result<Report, String> {
    let mut violations: BTreeSet<Violation> = BTreeSet::new();
    let mut graphs = Vec::new();

    // Declared graph: every dependency a manifest names.
    let declared = metadata(opts, &["--no-deps"])?;
    let members_by_id = members(&declared)?;
    let by_name: BTreeMap<&str, &Member> = members_by_id
        .values()
        .map(|m| (m.name.as_str(), m))
        .collect();
    graphs.push("declared".to_owned());
    for pkg in declared["packages"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        let Some(from) = members_by_id.get(pkg["id"].as_str().unwrap_or("")) else {
            continue;
        };
        let mut ports: BTreeSet<&str> = BTreeSet::new();
        for dep in pkg["dependencies"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            if dep["path"].is_null() {
                continue;
            }
            let to_name = dep["name"].as_str().unwrap_or("");
            let Some(to) = by_name.get(to_name) else {
                continue;
            };
            if to.layer == Layer::Ports {
                ports.insert(to_name);
            }
            let reason = if from.name == "nomos-core" {
                Some("nomos-core depends on no workspace crate")
            } else {
                edge_reason(from.layer, to.layer)
            };
            if let Some(reason) = reason {
                violations.insert(Violation {
                    graph: "declared".into(),
                    from: from.name.clone(),
                    to: to_name.to_owned(),
                    kind: kind_name(&dep["kind"]),
                    optional: dep["optional"].as_bool().unwrap_or(false),
                    target: dep["target"].as_str().map(str::to_owned),
                    rename: dep["rename"].as_str().map(str::to_owned),
                    reason: reason.into(),
                });
            }
        }
        if from.layer == Layer::Adapters && ports.len() != 1 {
            violations.insert(Violation {
                graph: "declared".into(),
                from: from.name.clone(),
                to: ports.iter().copied().collect::<Vec<_>>().join(", "),
                kind: "normal".into(),
                optional: false,
                target: None,
                rename: None,
                reason: format!(
                    "an adapter implements exactly one port; this one names {}",
                    ports.len()
                ),
            });
        }
    }

    // Resolved graph: the edges real builds see, per configuration.
    let mut configurations: Vec<(String, Vec<&str>)> = vec![
        ("resolved:default-features".into(), vec![]),
        ("resolved:all-features".into(), vec!["--all-features"]),
    ];
    for platform in PLATFORMS {
        configurations.push((
            format!("resolved:all-features:{platform}"),
            vec!["--all-features", "--filter-platform", platform],
        ));
    }
    for (label, extra) in configurations {
        let resolved = metadata(opts, &extra)?;
        let ids = members(&resolved)?;
        let nodes = resolved["resolve"]["nodes"]
            .as_array()
            .ok_or("metadata has no resolve graph")?;
        for node in nodes {
            let Some(from) = ids.get(node["id"].as_str().unwrap_or("")) else {
                continue;
            };
            for dep in node["deps"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
                let Some(to) = ids.get(dep["pkg"].as_str().unwrap_or("")) else {
                    continue;
                };
                let reason = if from.name == "nomos-core" {
                    Some("nomos-core depends on no workspace crate")
                } else {
                    edge_reason(from.layer, to.layer)
                };
                let Some(reason) = reason else {
                    continue;
                };
                for dk in dep["dep_kinds"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
                {
                    violations.insert(Violation {
                        graph: label.clone(),
                        from: from.name.clone(),
                        to: to.name.clone(),
                        kind: kind_name(&dk["kind"]),
                        optional: false,
                        target: dk["target"].as_str().map(str::to_owned),
                        rename: None,
                        reason: reason.into(),
                    });
                }
            }
        }
        graphs.push(label);
    }

    Ok(Report {
        manifest_path: opts.manifest_path.clone(),
        packages: members_by_id.len(),
        graphs_checked: graphs,
        violations: violations.into_iter().collect(),
    })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{Options, check};

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/layer-policy")
    }

    fn copy_tree(from: &Path, to: &Path) {
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                std::fs::create_dir_all(&target).unwrap();
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    /// The base workspace with one case's manifests laid over it, in a scratch directory.
    fn workspace(case: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nomos-layers-{}-{case}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        copy_tree(&fixtures().join("base"), &dir);
        if case != "allowed" {
            copy_tree(&fixtures().join("cases").join(case), &dir);
        }
        dir
    }

    fn run(case: &str) -> Vec<super::Violation> {
        let report = check(&Options {
            manifest_path: workspace(case).join("Cargo.toml"),
            locked: false,
            offline: true,
        })
        .unwrap();
        report.violations().to_vec()
    }

    #[test]
    fn the_real_workspace_conforms() {
        let report = check(&Options {
            manifest_path: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../Cargo.toml"),
            locked: true,
            offline: false,
        })
        .unwrap();
        assert!(report.violations().is_empty(), "{:#?}", report.violations());
    }

    #[test]
    fn the_allowed_fixture_passes_so_the_checker_cannot_pass_by_rejecting_everything() {
        assert!(run("allowed").is_empty());
    }

    #[test]
    fn a_direct_core_to_adapter_edge_is_rejected_in_both_graphs() {
        let found = run("direct");
        assert!(
            found.iter().any(|v| v.graph() == "declared"
                && v.from() == "fx-model"
                && v.to() == "fx-adapter")
        );
        assert!(
            found
                .iter()
                .any(|v| v.graph().starts_with("resolved") && v.to() == "fx-adapter")
        );
    }

    #[test]
    fn a_renamed_dependency_is_checked_by_its_real_package_name() {
        let found = run("renamed");
        assert!(found.iter().any(|v| v.from() == "fx-model"
            && v.to() == "fx-adapter"
            && v.rename.as_deref() == Some("adapter")));
    }

    #[test]
    fn an_optional_edge_is_caught_declared_and_under_all_features_but_not_by_default_features() {
        let found = run("optional");
        assert!(
            found
                .iter()
                .any(|v| v.graph() == "declared" && v.optional && v.to() == "fx-adapter")
        );
        assert!(
            found
                .iter()
                .any(|v| v.graph() == "resolved:all-features" && v.to() == "fx-adapter")
        );
        assert!(
            !found
                .iter()
                .any(|v| v.graph() == "resolved:default-features")
        );
    }

    #[test]
    fn a_target_specific_edge_is_caught() {
        let found = run("target-cfg");
        assert!(
            found
                .iter()
                .any(|v| v.graph() == "declared" && v.target.is_some() && v.to() == "fx-adapter")
        );
        assert!(
            found
                .iter()
                .any(|v| v.graph().contains("linux-gnu") && v.to() == "fx-adapter")
        );
    }

    #[test]
    fn a_build_dependency_is_caught() {
        let found = run("build");
        assert!(
            found
                .iter()
                .any(|v| v.kind() == "build" && v.from() == "fx-model" && v.to() == "fx-adapter")
        );
    }

    #[test]
    fn the_mock_adapter_as_a_dev_dependency_of_app_is_still_forbidden() {
        let found = run("dev");
        assert!(
            found
                .iter()
                .any(|v| v.kind() == "dev" && v.from() == "fx-app" && v.to() == "fx-adapter")
        );
    }

    #[test]
    fn an_adapter_naming_two_ports_is_rejected() {
        let found = run("two-ports");
        assert!(
            found
                .iter()
                .any(|v| v.from() == "fx-adapter" && v.reason.contains("exactly one port"))
        );
    }

    #[test]
    fn nothing_may_depend_on_a_bin_crate() {
        let found = run("bin-dep");
        assert!(
            found
                .iter()
                .any(|v| v.to() == "fx-bin" && v.reason.contains("bin crate"))
        );
    }

    #[test]
    fn core_may_not_depend_on_a_port() {
        let found = run("core-to-port");
        assert!(
            found
                .iter()
                .any(|v| v.from() == "fx-model" && v.to() == "fx-port")
        );
    }
}
