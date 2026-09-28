//! The dependency-policy check for the hexagonal layout (ADR 0000).
//!
//! Cargo checks that the edges which exist resolve. It does not check that an
//! edge is allowed. This module reads `cargo metadata` twice: the **declared**
//! graph (`--no-deps`), which lists every dependency a manifest names, whether
//! optional, feature-gated, target-specific, renamed, dev, or build, and the
//! **resolved** graph under default features, all features, and each supported
//! target, which lists the edges real builds see. A forbidden edge behind an
//! inactive feature is caught by the first; a forbidden path that only exists
//! once features and targets are applied is caught by the second.
//!
//! A package's layer is its directory, `crates/<layer>/<package>`. The policy:
//!
//! | Source | May depend on |
//! | --- | --- |
//! | `nomos-core` | no workspace crate |
//! | other `core/` | `core/` |
//! | `ports/` | `core/` |
//! | `app/` | `core/`, `ports/` |
//! | `adapters/` | `core/`, and exactly the port it implements |
//! | `bin/` | anything except a `bin/` crate |
//!
//! No crate depends on a `bin/` crate, which keeps composition roots and the
//! `nomos-xtask` tooling crate out of every production dependency graph. An
//! adapter implements the port whose name prefixes its own
//! (`nomos-<port>-<technology>`), and depends on it unconditionally: a normal,
//! non-optional dependency with no target condition. Normal, dev, and build dependencies are held
//! to the same policy: there are no exemptions by dependency kind. A declared
//! dependency is matched to a workspace crate by package name, not alias and
//! not source, so a rename or a registry dependency with a workspace crate's
//! name cannot slip past.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use serde_json::Value;

/// The workspace crate that depends on no other workspace crate.
const ROOT_CORE: &str = "nomos-core";

/// Targets a real build may use. The resolved graph is checked for each.
const PLATFORMS: [&str; 2] = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"];

/// A layer of the hexagon, from the package's directory.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Layer {
    /// `crates/core/`
    Core,
    /// `crates/ports/`
    Ports,
    /// `crates/app/`
    App,
    /// `crates/adapters/`
    Adapters,
    /// `crates/bin/`
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

    fn as_str(self) -> &'static str {
        match self {
            Layer::Core => "core",
            Layer::Ports => "ports",
            Layer::App => "app",
            Layer::Adapters => "adapters",
            Layer::Bin => "bin",
        }
    }
}

/// Which rule an edge breaks. The kebab-case form is the stable identifier.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Rule {
    /// `nomos-core` depends on a workspace crate.
    RootCoreHasWorkspaceDependency,
    /// A core crate depends outside `core/`.
    CoreDependsOutsideCore,
    /// A port depends outside `core/`.
    PortDependsOutsideCore,
    /// The application depends outside `core/` and `ports/`.
    AppDependsOutsideCoreAndPorts,
    /// An adapter depends outside `core/` and `ports/`.
    AdapterDependsOutsideCoreAndPorts,
    /// An adapter depends on a port other than the one it implements.
    AdapterDependsOnForeignPort,
    /// An adapter does not depend on the port it implements.
    AdapterMissingItsPort,
    /// An adapter's name does not name a port in the workspace.
    AdapterNamesNoPort,
    /// Something depends on a `bin/` crate.
    DependsOnBin,
}

impl Rule {
    fn reason(self) -> &'static str {
        match self {
            Rule::RootCoreHasWorkspaceDependency => "nomos-core depends on no workspace crate",
            Rule::CoreDependsOutsideCore => "core depends only on core",
            Rule::PortDependsOutsideCore => "ports depend only on core",
            Rule::AppDependsOutsideCoreAndPorts => "app depends only on core and ports",
            Rule::AdapterDependsOutsideCoreAndPorts => {
                "adapters depend only on core and the port they implement"
            }
            Rule::AdapterDependsOnForeignPort => {
                "an adapter depends only on the port it implements"
            }
            Rule::AdapterMissingItsPort => {
                "an adapter depends on the port it implements unconditionally: a normal, non-optional dependency with no target condition"
            }
            Rule::AdapterNamesNoPort => {
                "an adapter is named nomos-<port>-<technology> after a port in the workspace"
            }
            Rule::DependsOnBin => {
                "nothing depends on a bin crate; composition roots and tooling stay out of the production graph"
            }
        }
    }
}

/// One forbidden edge, with everything needed to find it in a manifest.
#[derive(Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Violation {
    rule: Rule,
    graph: String,
    source: String,
    source_layer: Layer,
    dependency: Option<String>,
    dependency_layer: Option<Layer>,
    kind: Option<String>,
    optional: bool,
    target: Option<String>,
    rename: Option<String>,
    reason: &'static str,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {} ({})",
            serde_plain(&self.rule),
            self.source,
            self.source_layer.as_str()
        )?;
        if let (Some(dep), Some(layer)) = (&self.dependency, self.dependency_layer) {
            write!(f, " -> {dep} ({})", layer.as_str())?;
        }
        if let Some(kind) = &self.kind {
            write!(f, " kind={kind}")?;
        }
        if self.optional {
            write!(f, " optional")?;
        }
        if let Some(target) = &self.target {
            write!(f, " target={target}")?;
        }
        if let Some(rename) = &self.rename {
            write!(f, " as={rename}")?;
        }
        write!(f, " graph={}: {}", self.graph, self.reason)
    }
}

fn serde_plain(rule: &Rule) -> String {
    serde_json::to_value(rule)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

#[cfg(test)]
impl Violation {
    pub(crate) fn rule(&self) -> Rule {
        self.rule
    }
    pub(crate) fn graph(&self) -> &str {
        &self.graph
    }
    pub(crate) fn source(&self) -> &str {
        &self.source
    }
    pub(crate) fn dependency(&self) -> Option<&str> {
        self.dependency.as_deref()
    }
    pub(crate) fn kind(&self) -> Option<&str> {
        self.kind.as_deref()
    }
}

/// The check's report.
#[derive(Serialize)]
pub(crate) struct Report {
    manifest_path: PathBuf,
    packages: BTreeMap<String, Layer>,
    graphs_checked: Vec<String>,
    violations: Vec<Violation>,
}

impl Report {
    /// The forbidden edges found, sorted and de-duplicated.
    pub(crate) fn violations(&self) -> &[Violation] {
        &self.violations
    }

    /// Every workspace package and its layer.
    #[cfg(test)]
    pub(crate) fn packages(&self) -> &BTreeMap<String, Layer> {
        &self.packages
    }

    /// The graphs that were checked.
    #[cfg(test)]
    pub(crate) fn graphs_checked(&self) -> &[String] {
        &self.graphs_checked
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

#[derive(Clone)]
struct Member {
    name: String,
    layer: Layer,
}

pub(crate) fn metadata(opts: &Options, extra: &[&str]) -> Result<Value, String> {
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

/// Workspace members by package ID.
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

/// The port an adapter implements: the longest port name that prefixes the adapter's.
fn implemented_port<'a>(adapter: &str, ports: &'a BTreeSet<String>) -> Option<&'a str> {
    ports
        .iter()
        .filter(|port| adapter.starts_with(&format!("{port}-")))
        .max_by_key(|port| port.len())
        .map(String::as_str)
}

/// The rule an edge breaks, if any. `port` is the adapter's implemented port.
fn edge_rule(from: &Member, to: &Member, port: Option<&str>) -> Option<Rule> {
    if to.layer == Layer::Bin {
        return Some(Rule::DependsOnBin);
    }
    if from.name == ROOT_CORE {
        return Some(Rule::RootCoreHasWorkspaceDependency);
    }
    match (from.layer, to.layer) {
        (Layer::Core, Layer::Core) => None,
        (Layer::Core, _) => Some(Rule::CoreDependsOutsideCore),
        (Layer::Ports, Layer::Core) => None,
        (Layer::Ports, _) => Some(Rule::PortDependsOutsideCore),
        (Layer::App, Layer::Core | Layer::Ports) => None,
        (Layer::App, _) => Some(Rule::AppDependsOutsideCoreAndPorts),
        (Layer::Adapters, Layer::Core) => None,
        (Layer::Adapters, Layer::Ports) => match port {
            Some(port) if port == to.name => None,
            // An adapter whose name names no port is reported once, on the package.
            None => None,
            Some(_) => Some(Rule::AdapterDependsOnForeignPort),
        },
        (Layer::Adapters, _) => Some(Rule::AdapterDependsOutsideCoreAndPorts),
        (Layer::Bin, _) => None,
    }
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
    let ports: BTreeSet<String> = members_by_id
        .values()
        .filter(|m| m.layer == Layer::Ports)
        .map(|m| m.name.clone())
        .collect();
    let port_of = |m: &Member| {
        (m.layer == Layer::Adapters)
            .then(|| implemented_port(&m.name, &ports))
            .flatten()
            .map(str::to_owned)
    };
    graphs.push("declared".to_owned());
    for pkg in declared["packages"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        let Some(from) = members_by_id.get(pkg["id"].as_str().unwrap_or("")) else {
            continue;
        };
        let port = port_of(from);
        let mut depends_on_its_port = false;
        for dep in pkg["dependencies"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            let to_name = dep["name"].as_str().unwrap_or("");
            let Some(to) = by_name.get(to_name) else {
                continue;
            };
            let kind = kind_name(&dep["kind"]);
            // The port an adapter implements must be present in every build of the
            // adapter: a normal dependency, not optional, with no target condition.
            // An optional or target-specific declaration names the port without
            // guaranteeing the edge exists.
            if port.as_deref() == Some(to_name)
                && kind == "normal"
                && !dep["optional"].as_bool().unwrap_or(false)
                && dep["target"].is_null()
            {
                depends_on_its_port = true;
            }
            if let Some(rule) = edge_rule(from, to, port.as_deref()) {
                violations.insert(Violation {
                    rule,
                    graph: "declared".into(),
                    source: from.name.clone(),
                    source_layer: from.layer,
                    dependency: Some(to.name.clone()),
                    dependency_layer: Some(to.layer),
                    kind: Some(kind),
                    optional: dep["optional"].as_bool().unwrap_or(false),
                    target: dep["target"].as_str().map(str::to_owned),
                    rename: dep["rename"].as_str().map(str::to_owned),
                    reason: rule.reason(),
                });
            }
        }
        if from.layer == Layer::Adapters {
            let missing = match &port {
                None => Some((Rule::AdapterNamesNoPort, None)),
                Some(port) if !depends_on_its_port => {
                    Some((Rule::AdapterMissingItsPort, Some(port.clone())))
                }
                Some(_) => None,
            };
            if let Some((rule, dependency)) = missing {
                violations.insert(Violation {
                    rule,
                    graph: "declared".into(),
                    source: from.name.clone(),
                    source_layer: from.layer,
                    dependency_layer: dependency.as_ref().map(|_| Layer::Ports),
                    dependency,
                    kind: None,
                    optional: false,
                    target: None,
                    rename: None,
                    reason: rule.reason(),
                });
            }
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
            let port = port_of(from);
            for dep in node["deps"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
                let Some(to) = ids.get(dep["pkg"].as_str().unwrap_or("")) else {
                    continue;
                };
                let Some(rule) = edge_rule(from, to, port.as_deref()) else {
                    continue;
                };
                for dk in dep["dep_kinds"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
                {
                    violations.insert(Violation {
                        rule,
                        graph: label.clone(),
                        source: from.name.clone(),
                        source_layer: from.layer,
                        dependency: Some(to.name.clone()),
                        dependency_layer: Some(to.layer),
                        kind: Some(kind_name(&dk["kind"])),
                        optional: false,
                        target: dk["target"].as_str().map(str::to_owned),
                        rename: None,
                        reason: rule.reason(),
                    });
                }
            }
        }
        graphs.push(label);
    }

    Ok(Report {
        manifest_path: opts.manifest_path.clone(),
        packages: members_by_id
            .values()
            .map(|m| (m.name.clone(), m.layer))
            .collect(),
        graphs_checked: graphs,
        violations: violations.into_iter().collect(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    use super::{Options, Rule, Violation, check};

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

    /// The base workspace, optionally with one case's manifests laid over it,
    /// in a scratch directory so Cargo writes nothing into the repository.
    fn workspace(label: &str, case: Option<&str>) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nomos-layers-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        copy_tree(&fixtures().join("base"), &dir);
        if let Some(case) = case {
            copy_tree(&fixtures().join("cases").join(case), &dir);
        }
        dir
    }

    fn check_dir(dir: &Path) -> Vec<Violation> {
        check(&Options {
            manifest_path: dir.join("Cargo.toml"),
            locked: false,
            offline: true,
        })
        .unwrap()
        .violations()
        .to_vec()
    }

    fn run(case: &str) -> Vec<Violation> {
        check_dir(&workspace(case, Some(case)))
    }

    fn rules(found: &[Violation]) -> BTreeSet<Rule> {
        found.iter().map(Violation::rule).collect()
    }

    /// The case fails, only for the expected rules, and names the expected edge.
    fn assert_case(
        case: &str,
        expected: &[Rule],
        source: &str,
        dependency: Option<&str>,
        kind: Option<&str>,
    ) -> Vec<Violation> {
        let found = run(case);
        assert_eq!(
            rules(&found),
            expected.iter().copied().collect(),
            "{case}: {found:#?}"
        );
        assert!(
            found.iter().any(|v| v.graph() == "declared"
                && v.source() == source
                && v.dependency() == dependency
                && v.kind() == kind),
            "{case}: no declared violation {source} -> {dependency:?} kind {kind:?}: {found:#?}"
        );
        found
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
        assert_eq!(report.packages().len(), 16);
        assert_eq!(report.graphs_checked().len(), 5);
    }

    #[test]
    fn the_allowed_fixture_passes_so_the_checker_cannot_pass_by_rejecting_everything() {
        let dir = workspace("allowed", None);
        let report = check(&Options {
            manifest_path: dir.join("Cargo.toml"),
            locked: false,
            offline: true,
        })
        .unwrap();
        assert!(report.violations().is_empty(), "{:#?}", report.violations());
        assert_eq!(report.packages().len(), 11);
        assert_eq!(report.graphs_checked().len(), 5);
    }

    #[test]
    fn core_to_adapter_is_rejected_in_declared_and_resolved_graphs() {
        let found = assert_case(
            "direct-core-adapter",
            &[Rule::CoreDependsOutsideCore],
            "nomos-canon",
            Some("nomos-substrate-linux"),
            Some("normal"),
        );
        assert!(
            found
                .iter()
                .any(|v| v.graph() == "resolved:default-features")
        );
    }

    #[test]
    fn port_to_adapter_is_rejected() {
        assert_case(
            "direct-port-adapter",
            &[Rule::PortDependsOutsideCore],
            "nomos-cipher",
            Some("nomos-substrate-linux"),
            Some("normal"),
        );
    }

    #[test]
    fn app_to_adapter_is_rejected() {
        assert_case(
            "direct-app-adapter",
            &[Rule::AppDependsOutsideCoreAndPorts],
            "nomos-app",
            Some("nomos-substrate-mock"),
            Some("normal"),
        );
    }

    #[test]
    fn a_renamed_dependency_is_checked_by_its_package_not_its_alias() {
        let found = assert_case(
            "renamed",
            &[Rule::CoreDependsOutsideCore],
            "nomos-canon",
            Some("nomos-substrate-linux"),
            Some("normal"),
        );
        assert!(
            found
                .iter()
                .any(|v| v.rename.as_deref() == Some("linux_backend"))
        );
    }

    #[test]
    fn an_inactive_optional_dependency_is_caught_declared_and_under_all_features() {
        let found = assert_case(
            "optional",
            &[Rule::AppDependsOutsideCoreAndPorts],
            "nomos-app",
            Some("nomos-substrate-linux"),
            Some("normal"),
        );
        assert!(found.iter().any(|v| v.graph() == "declared" && v.optional));
        assert!(found.iter().any(|v| v.graph() == "resolved:all-features"));
        assert!(
            !found
                .iter()
                .any(|v| v.graph() == "resolved:default-features"),
            "an inactive optional edge is not in the default resolution"
        );
    }

    #[test]
    fn a_target_specific_dependency_is_caught() {
        let found = assert_case(
            "target-cfg",
            &[Rule::AppDependsOutsideCoreAndPorts],
            "nomos-app",
            Some("nomos-cipher-vault"),
            Some("normal"),
        );
        assert!(
            found
                .iter()
                .any(|v| v.graph() == "declared" && v.target.as_deref() == Some("cfg(unix)"))
        );
        assert!(
            found
                .iter()
                .any(|v| v.graph() == "resolved:all-features:x86_64-unknown-linux-gnu")
        );
    }

    #[test]
    fn a_build_dependency_is_held_to_the_same_rule() {
        assert_case(
            "build",
            &[Rule::CoreDependsOutsideCore],
            "nomos-canon",
            Some("nomos-substrate-linux"),
            Some("build"),
        );
    }

    #[test]
    fn a_dev_dependency_is_held_to_the_same_rule() {
        assert_case(
            "dev",
            &[Rule::AppDependsOutsideCoreAndPorts],
            "nomos-app",
            Some("nomos-substrate-mock"),
            Some("dev"),
        );
    }

    #[test]
    fn nomos_core_may_not_depend_on_another_core_crate() {
        assert_case(
            "root-core",
            &[Rule::RootCoreHasWorkspaceDependency],
            "nomos-core",
            Some("nomos-ids"),
            Some("normal"),
        );
    }

    #[test]
    fn an_adapter_depending_on_another_port_as_well_is_rejected() {
        assert_case(
            "adapter-extra-port",
            &[Rule::AdapterDependsOnForeignPort],
            "nomos-substrate-linux",
            Some("nomos-cipher"),
            Some("normal"),
        );
    }

    #[test]
    fn an_adapter_depending_on_the_wrong_port_instead_is_rejected() {
        assert_case(
            "adapter-wrong-port",
            &[
                Rule::AdapterDependsOnForeignPort,
                Rule::AdapterMissingItsPort,
            ],
            "nomos-substrate-linux",
            Some("nomos-cipher"),
            Some("normal"),
        );
    }

    #[test]
    fn an_adapter_whose_own_port_is_optional_is_missing_it() {
        assert_case(
            "adapter-own-port-optional",
            &[Rule::AdapterMissingItsPort],
            "nomos-substrate-linux",
            Some("nomos-substrate"),
            None,
        );
    }

    #[test]
    fn an_adapter_whose_own_port_is_target_specific_is_missing_it() {
        assert_case(
            "adapter-own-port-target",
            &[Rule::AdapterMissingItsPort],
            "nomos-substrate-linux",
            Some("nomos-substrate"),
            None,
        );
    }

    #[test]
    fn an_adapter_named_after_no_port_is_rejected() {
        assert_case(
            "adapter-names-no-port",
            &[Rule::AdapterNamesNoPort],
            "nomos-telemetry-otel",
            None,
            None,
        );
    }

    #[test]
    fn a_production_crate_may_not_depend_on_the_tooling_crate() {
        assert_case(
            "app-to-tooling",
            &[Rule::DependsOnBin],
            "nomos-app",
            Some("nomos-xtask"),
            Some("normal"),
        );
    }

    #[test]
    fn a_composition_root_may_not_depend_on_the_tooling_crate() {
        assert_case(
            "cell-to-tooling",
            &[Rule::DependsOnBin],
            "nomos-cell",
            Some("nomos-xtask"),
            Some("normal"),
        );
    }

    #[test]
    fn restoring_the_valid_manifest_makes_the_workspace_pass_again() {
        let dir = workspace("restored", Some("direct-app-adapter"));
        assert!(!check_dir(&dir).is_empty());
        let app = "crates/app/nomos-app/Cargo.toml";
        std::fs::copy(fixtures().join("base").join(app), dir.join(app)).unwrap();
        assert!(check_dir(&dir).is_empty());
    }

    #[test]
    fn every_case_directory_has_a_test() {
        let cases: BTreeSet<String> = std::fs::read_dir(fixtures().join("cases"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        let source = include_str!("layers.rs");
        for case in &cases {
            assert!(
                source.contains(&format!("\"{case}\"")),
                "fixture case {case} has no test"
            );
        }
        assert_eq!(cases.len(), 16);
    }
}
