//! The core purity check (ADR 0016): a domain crate carries no ambient effect.
//!
//! The dependency rule (`check-layers`) keeps adapters out of `core/`. It says
//! nothing about a registry crate: `tokio`, `rand`, or a build script would
//! pass it. This check reads `crates/core/PURITY.toml` and holds every
//! workspace crate under `crates/core/` to it, over `cargo metadata`:
//!
//! - no build script, from the `custom-build` target Cargo reports;
//! - every declared normal or build dependency outside the workspace is
//!   allowlisted, with default features off when the policy says so and no
//!   feature the policy does not name, and is in no denied class;
//! - every crate the resolved graph reaches from a core crate, under all
//!   features, is in no denied class, and is allowlisted directly or as a
//!   declared transitive dependency;
//! - no `[allow]` entry, and no `transitive` list, names a crate in a denied
//!   class: a denied class is never admitted, directly or transitively, and a
//!   policy that says otherwise contradicts itself;
//! - the crate root declares `#![no_std]` and denies the required lints as
//!   inner attributes, and the manifest inherits the workspace lints, which
//!   forbid `unsafe`.
//!
//! Dev-dependencies are exempt: they build test binaries, never the artifact a
//! composition root links. Workspace crates are not checked here; the layer
//! check owns those edges. A `#![cfg_attr(..., no_std)]` or a lint denied only
//! under a `cfg_attr` does not satisfy the policy: the attribute must hold in
//! every build. The check reads attributes and manifests; it does not compile.
//! What it cannot see, and the compiler checks instead, is listed in
//! `docs/formal/core-purity.md`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::layers::{Options, metadata};

/// The policy file, relative to the workspace root.
pub(crate) const POLICY_PATH: &str = "crates/core/PURITY.toml";

/// Why a core crate fails the policy. The kebab-case form is the stable identifier.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PurityCode {
    /// `crates/core/PURITY.toml` is not there.
    PolicyMissing,
    /// The policy does not parse or has the wrong shape.
    PolicyMalformed,
    /// The policy lists a crate the workspace does not have under `crates/core/`.
    PolicyNamesUnknownCrate,
    /// An `[allow]` entry, or its `transitive` list, names a crate in a denied class.
    PolicyAllowsDeniedClass,
    /// A workspace crate under `crates/core/` is not in the policy.
    CoreCrateUnlisted,
    /// A core crate has a build script.
    BuildScript,
    /// A core crate names a dependency in a denied class.
    DependencyDeniedClass,
    /// A core crate names a dependency the policy does not allow.
    DependencyNotAllowlisted,
    /// An allowed dependency keeps default features or enables an unlisted feature.
    DependencyFeatures,
    /// The resolved graph reaches a crate in a denied class, whatever `[allow]` says.
    TransitiveDependencyDeniedClass,
    /// The resolved graph reaches a crate the policy does not name.
    TransitiveDependencyNotAllowlisted,
    /// The crate root does not declare `#![no_std]`.
    NoStdMissing,
    /// The crate root does not deny a required lint.
    RequiredLintMissing,
    /// The manifest does not inherit the workspace lints.
    LintsNotFromWorkspace,
}

impl PurityCode {
    /// The stable identifier.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            PurityCode::PolicyMissing => "purity-policy-missing",
            PurityCode::PolicyMalformed => "purity-policy-malformed",
            PurityCode::PolicyNamesUnknownCrate => "purity-policy-names-unknown-crate",
            PurityCode::PolicyAllowsDeniedClass => "purity-policy-allows-denied-class",
            PurityCode::CoreCrateUnlisted => "core-crate-unlisted",
            PurityCode::BuildScript => "core-build-script",
            PurityCode::DependencyDeniedClass => "core-dependency-denied-class",
            PurityCode::DependencyNotAllowlisted => "core-dependency-not-allowlisted",
            PurityCode::DependencyFeatures => "core-dependency-features",
            PurityCode::TransitiveDependencyDeniedClass => {
                "core-transitive-dependency-denied-class"
            }
            PurityCode::TransitiveDependencyNotAllowlisted => {
                "core-transitive-dependency-not-allowlisted"
            }
            PurityCode::NoStdMissing => "core-no-std-missing",
            PurityCode::RequiredLintMissing => "core-required-lint-missing",
            PurityCode::LintsNotFromWorkspace => "core-lints-not-workspace",
        }
    }
}

/// One failure, with the crate and dependency it concerns.
#[derive(Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Violation {
    code: PurityCode,
    package: String,
    dependency: Option<String>,
    class: Option<String>,
    detail: String,
}

impl Violation {
    fn new(code: PurityCode, package: &str, detail: impl Into<String>) -> Self {
        Violation {
            code,
            package: package.to_owned(),
            dependency: None,
            class: None,
            detail: detail.into(),
        }
    }

    /// The stable reason.
    #[cfg(test)]
    pub(crate) fn code(&self) -> PurityCode {
        self.code
    }

    /// The dependency the failure names, if any.
    #[cfg(test)]
    pub(crate) fn dependency(&self) -> Option<&str> {
        self.dependency.as_deref()
    }

    /// The denied class, if any.
    #[cfg(test)]
    pub(crate) fn class(&self) -> Option<&str> {
        self.class.as_deref()
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code.as_str(), self.package)?;
        if let Some(dep) = &self.dependency {
            write!(f, " -> {dep}")?;
        }
        if let Some(class) = &self.class {
            write!(f, " class={class}")?;
        }
        write!(f, ": {}", self.detail)
    }
}

/// The check's report.
#[derive(Serialize)]
pub(crate) struct Report {
    policy_path: PathBuf,
    crates: Vec<String>,
    violations: Vec<Violation>,
}

impl Report {
    /// The failures found, sorted and de-duplicated.
    pub(crate) fn violations(&self) -> &[Violation] {
        &self.violations
    }

    /// The core crates the policy was applied to.
    #[cfg(test)]
    pub(crate) fn crates(&self) -> &[String] {
        &self.crates
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    policy: Rules,
    crates: BTreeMap<String, CrateEntry>,
    #[serde(default)]
    allow: BTreeMap<String, Allow>,
    #[serde(default)]
    deny: BTreeMap<String, Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rules {
    no_std: bool,
    build_scripts: bool,
    required_lints: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CrateEntry {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Allow {
    default_features: bool,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default)]
    transitive: Vec<String>,
    #[allow(
        dead_code,
        reason = "the reason is for the reviewer; the policy requires it"
    )]
    reason: String,
}

struct CoreCrate {
    id: String,
    name: String,
    manifest_path: PathBuf,
    lib_path: Option<PathBuf>,
    has_build_script: bool,
}

/// The workspace members under `crates/core/`.
fn core_crates(meta: &Value) -> Result<Vec<CoreCrate>, String> {
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
    let mut out = Vec::new();
    for pkg in meta["packages"]
        .as_array()
        .ok_or("metadata has no packages")?
    {
        let id = pkg["id"].as_str().unwrap_or("");
        if !ids.contains(id) {
            continue;
        }
        let manifest_path = PathBuf::from(pkg["manifest_path"].as_str().unwrap_or(""));
        if !manifest_path.starts_with(root.join("crates").join("core")) {
            continue;
        }
        let targets = pkg["targets"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        let kinds = |t: &Value| -> Vec<String> {
            t["kind"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[])
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        };
        let has_build_script = targets
            .iter()
            .any(|t| kinds(t).iter().any(|k| k == "custom-build"));
        let lib_path = targets
            .iter()
            .find(|t| kinds(t).iter().any(|k| k == "lib" || k == "rlib"))
            .and_then(|t| t["src_path"].as_str())
            .map(PathBuf::from);
        out.push(CoreCrate {
            id: id.to_owned(),
            name: pkg["name"].as_str().unwrap_or("").to_owned(),
            manifest_path,
            lib_path,
            has_build_script,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Inner attributes (`#![...]`) of a Rust source, comments stripped, each as
/// the text between the brackets with whitespace collapsed.
fn inner_attributes(source: &str) -> Vec<String> {
    let stripped: String = source
        .lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = Vec::new();
    let mut rest = stripped.as_str();
    while let Some(start) = rest.find("#![") {
        let body = &rest[start + 3..];
        let mut depth = 1usize;
        let mut end = None;
        for (i, c) in body.char_indices() {
            match c {
                '[' | '(' => depth += 1,
                ']' | ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else { break };
        out.push(body[..end].split_whitespace().collect::<Vec<_>>().join(" "));
        rest = &body[end + 1..];
    }
    out
}

/// The lints a source denies or forbids at its root, unconditionally.
fn denied_lints(attributes: &[String]) -> BTreeSet<String> {
    attributes
        .iter()
        .filter_map(|a| {
            a.strip_prefix("deny(")
                .or_else(|| a.strip_prefix("forbid("))
                .and_then(|inner| inner.strip_suffix(')'))
        })
        .flat_map(|inner| inner.split(','))
        .map(|lint| lint.split_whitespace().collect::<String>())
        .filter(|lint| !lint.is_empty())
        .collect()
}

fn read_policy(path: &Path) -> Result<Policy, Violation> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        let code = if e.kind() == std::io::ErrorKind::NotFound {
            PurityCode::PolicyMissing
        } else {
            PurityCode::PolicyMalformed
        };
        Violation::new(code, "-", format!("{}: {e}", path.display()))
    })?;
    toml::from_str(&text).map_err(|e| {
        Violation::new(
            PurityCode::PolicyMalformed,
            "-",
            format!("{}: {}", path.display(), e.message()),
        )
    })
}

fn kind_name(kind: &Value) -> &str {
    kind.as_str().unwrap_or("normal")
}

/// Runs the check against the workspace `opts` names.
pub(crate) fn check(opts: &Options) -> Result<Report, String> {
    let declared = metadata(opts, &["--no-deps"])?;
    let root = PathBuf::from(
        declared["workspace_root"]
            .as_str()
            .ok_or("metadata has no workspace_root")?,
    );
    let policy_path = root.join(POLICY_PATH);
    let crates = core_crates(&declared)?;
    let crate_names: Vec<String> = crates.iter().map(|c| c.name.clone()).collect();
    let mut violations: BTreeSet<Violation> = BTreeSet::new();

    let policy = match read_policy(&policy_path) {
        Ok(policy) => policy,
        Err(violation) => {
            violations.insert(violation);
            return Ok(Report {
                policy_path,
                crates: crate_names,
                violations: violations.into_iter().collect(),
            });
        }
    };

    for name in policy.crates.keys() {
        if !crate_names.contains(name) {
            violations.insert(Violation::new(
                PurityCode::PolicyNamesUnknownCrate,
                name,
                "listed in the policy but not a workspace crate under crates/core/",
            ));
        }
    }
    for name in &crate_names {
        if !policy.crates.contains_key(name) {
            violations.insert(Violation::new(
                PurityCode::CoreCrateUnlisted,
                name,
                format!("a core crate that {POLICY_PATH} does not list"),
            ));
        }
    }

    let members: BTreeSet<&str> = declared["workspace_members"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let member_names: BTreeSet<String> = declared["packages"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter(|p| members.contains(p["id"].as_str().unwrap_or("")))
        .filter_map(|p| p["name"].as_str().map(str::to_owned))
        .collect();
    let class_of = |dep: &str| -> Option<&str> {
        policy
            .deny
            .iter()
            .find(|(_, names)| names.iter().any(|n| n == dep))
            .map(|(class, _)| class.as_str())
    };
    let transitively_allowed: BTreeSet<&str> = policy
        .allow
        .values()
        .flat_map(|a| a.transitive.iter().map(String::as_str))
        .collect();

    // A denied class is never admitted. An allow entry or a transitive list
    // that names one contradicts the policy's own [deny], and is rejected
    // here; the graph walks below reject the crate itself wherever it is
    // reached, so the contradiction cannot admit it either way.
    for (allowed, entry) in &policy.allow {
        let named = std::iter::once(allowed).chain(&entry.transitive);
        for name in named {
            if let Some(class) = class_of(name) {
                let place = if name == allowed {
                    format!("[allow.{allowed}]")
                } else {
                    format!("[allow.{allowed}].transitive")
                };
                violations.insert(Violation {
                    code: PurityCode::PolicyAllowsDeniedClass,
                    package: allowed.clone(),
                    dependency: Some(name.clone()),
                    class: Some(class.to_owned()),
                    detail: format!(
                        "{place} names {name}, in the denied class {class}; a denied class is never allowed"
                    ),
                });
            }
        }
    }

    // Declared dependencies and manifests. A direct dependency is reported
    // here, by its own code, and not again by the transitive walk below.
    let mut direct: BTreeSet<(String, String)> = BTreeSet::new();
    for pkg in declared["packages"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        let Some(krate) = crates
            .iter()
            .find(|c| c.id == pkg["id"].as_str().unwrap_or(""))
        else {
            continue;
        };
        if !policy.policy.build_scripts && krate.has_build_script {
            violations.insert(Violation::new(
                PurityCode::BuildScript,
                &krate.name,
                "a build script runs arbitrary code at compile time; core crates have none",
            ));
        }
        for dep in pkg["dependencies"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            let dep_name = dep["name"].as_str().unwrap_or("");
            if kind_name(&dep["kind"]) == "dev" || member_names.contains(dep_name) {
                continue;
            }
            direct.insert((krate.name.clone(), dep_name.to_owned()));
            let mut violation = |code, class: Option<&str>, detail: String| {
                violations.insert(Violation {
                    code,
                    package: krate.name.clone(),
                    dependency: Some(dep_name.to_owned()),
                    class: class.map(str::to_owned),
                    detail,
                });
            };
            if let Some(class) = class_of(dep_name) {
                violation(
                    PurityCode::DependencyDeniedClass,
                    Some(class),
                    format!("{dep_name} is in the denied class {class}"),
                );
                continue;
            }
            let Some(allow) = policy.allow.get(dep_name) else {
                violation(
                    PurityCode::DependencyNotAllowlisted,
                    None,
                    format!("{dep_name} is not in [allow] of {POLICY_PATH}"),
                );
                continue;
            };
            let uses_default = dep["uses_default_features"].as_bool().unwrap_or(true);
            if uses_default && !allow.default_features {
                violation(
                    PurityCode::DependencyFeatures,
                    None,
                    format!(
                        "{dep_name} keeps its default features; the policy requires default-features = false"
                    ),
                );
            }
            let extra: Vec<&str> = dep["features"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[])
                .iter()
                .filter_map(Value::as_str)
                .filter(|f| !allow.features.iter().any(|a| a == f))
                .collect();
            if !extra.is_empty() {
                violation(
                    PurityCode::DependencyFeatures,
                    None,
                    format!(
                        "{dep_name} enables features the policy does not list: {}",
                        extra.join(", ")
                    ),
                );
            }
        }

        // The manifest inherits the workspace lints, which forbid unsafe.
        let manifest_text = std::fs::read_to_string(&krate.manifest_path)
            .map_err(|e| format!("{}: {e}", krate.manifest_path.display()))?;
        let manifest: toml::Table = toml::from_str(&manifest_text)
            .map_err(|e| format!("{}: {}", krate.manifest_path.display(), e.message()))?;
        let inherits = manifest
            .get("lints")
            .and_then(|l| l.get("workspace"))
            .and_then(toml::Value::as_bool)
            .unwrap_or(false);
        if !inherits {
            violations.insert(Violation::new(
                PurityCode::LintsNotFromWorkspace,
                &krate.name,
                "the manifest does not set [lints] workspace = true",
            ));
        }

        // The crate root: #![no_std] and the required lints, unconditionally.
        let Some(lib_path) = &krate.lib_path else {
            continue;
        };
        let source = std::fs::read_to_string(lib_path)
            .map_err(|e| format!("{}: {e}", lib_path.display()))?;
        let attributes = inner_attributes(&source);
        if policy.policy.no_std && !attributes.iter().any(|a| a == "no_std") {
            violations.insert(Violation::new(
                PurityCode::NoStdMissing,
                &krate.name,
                format!("{} does not declare #![no_std]", lib_path.display()),
            ));
        }
        let denied = denied_lints(&attributes);
        for lint in &policy.policy.required_lints {
            if !denied.contains(lint) {
                violations.insert(Violation::new(
                    PurityCode::RequiredLintMissing,
                    &krate.name,
                    format!(
                        "{} does not deny {lint} at the crate root",
                        lib_path.display()
                    ),
                ));
            }
        }
    }

    // Resolved graph under all features: everything a core crate can pull in.
    let resolved = metadata(opts, &["--all-features"])?;
    let packages: BTreeMap<&str, &str> = resolved["packages"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|p| Some((p["id"].as_str()?, p["name"].as_str()?)))
        .collect();
    let nodes: BTreeMap<&str, &Value> = resolved["resolve"]["nodes"]
        .as_array()
        .ok_or("metadata has no resolve graph")?
        .iter()
        .filter_map(|n| Some((n["id"].as_str()?, n)))
        .collect();
    for krate in &crates {
        // Breadth-first over normal and build edges, remembering the direct
        // dependency each crate was reached through.
        let mut queue: VecDeque<(&str, Option<&str>)> = VecDeque::from([(krate.id.as_str(), None)]);
        let mut seen: BTreeSet<&str> = BTreeSet::from([krate.id.as_str()]);
        while let Some((id, via)) = queue.pop_front() {
            let Some(node) = nodes.get(id) else { continue };
            for dep in node["deps"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
                let kinds = dep["dep_kinds"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                if !kinds.iter().any(|k| kind_name(&k["kind"]) != "dev") {
                    continue;
                }
                let Some(pkg) = dep["pkg"].as_str() else {
                    continue;
                };
                if !seen.insert(pkg) {
                    continue;
                }
                let name = packages.get(pkg).copied().unwrap_or("");
                let via = via.or(Some(name));
                if member_names.contains(name) {
                    queue.push_back((pkg, via));
                    continue;
                }
                let declared_here = direct.contains(&(krate.name.clone(), name.to_owned()));
                // The class is checked before the allowlist, as for a
                // declared dependency: no allow entry admits a denied class.
                // A declared edge to one is already reported above.
                if let Some(class) = class_of(name) {
                    if !declared_here {
                        violations.insert(Violation {
                            code: PurityCode::TransitiveDependencyDeniedClass,
                            package: krate.name.clone(),
                            dependency: Some(name.to_owned()),
                            class: Some(class.to_owned()),
                            detail: format!(
                                "reached through {} under all features; {name} is in the denied class {class}, which no allow entry admits",
                                via.unwrap_or(name)
                            ),
                        });
                    }
                    queue.push_back((pkg, via));
                    continue;
                }
                let allowed = policy.allow.contains_key(name)
                    || transitively_allowed.contains(name)
                    || declared_here;
                if !allowed {
                    violations.insert(Violation {
                        code: PurityCode::TransitiveDependencyNotAllowlisted,
                        package: krate.name.clone(),
                        dependency: Some(name.to_owned()),
                        class: None,
                        detail: format!(
                            "reached through {} under all features; not in [allow] and not a declared transitive dependency",
                            via.unwrap_or(name)
                        ),
                    });
                }
                queue.push_back((pkg, via));
            }
        }
    }

    Ok(Report {
        policy_path,
        crates: crate_names,
        violations: violations.into_iter().collect(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    use super::{PurityCode, Violation, check, denied_lints, inner_attributes};
    use crate::layers::Options;

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/core-purity")
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

    fn workspace(label: &str, case: Option<&str>) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nomos-purity-{}-{label}", std::process::id()));
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

    fn codes(found: &[Violation]) -> BTreeSet<PurityCode> {
        found.iter().map(Violation::code).collect()
    }

    /// The case fails only for the expected codes, and names the expected crate and dependency.
    fn assert_case(
        case: &str,
        expected: &[PurityCode],
        package: &str,
        dependency: Option<&str>,
    ) -> Vec<Violation> {
        let found = check_dir(&workspace(case, Some(case)));
        assert_eq!(
            codes(&found),
            expected.iter().copied().collect(),
            "{case}: {found:#?}"
        );
        assert!(
            found
                .iter()
                .any(|v| v.package == package && v.dependency() == dependency),
            "{case}: no violation on {package} -> {dependency:?}: {found:#?}"
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
        assert_eq!(report.crates(), ["nomos-canon", "nomos-core", "nomos-warp"]);
    }

    #[test]
    fn the_allowed_fixture_passes_so_the_checker_cannot_pass_by_rejecting_everything() {
        let report = check(&Options {
            manifest_path: workspace("allowed", None).join("Cargo.toml"),
            locked: false,
            offline: true,
        })
        .unwrap();
        assert!(report.violations().is_empty(), "{:#?}", report.violations());
        assert_eq!(report.crates(), ["nomos-canon", "nomos-core"]);
    }

    #[test]
    fn a_dev_dependency_on_a_denied_crate_is_exempt() {
        let found = check_dir(&workspace(
            "dev-dependency-exempt",
            Some("dev-dependency-exempt"),
        ));
        assert!(found.is_empty(), "{found:#?}");
    }

    #[test]
    fn a_randomness_crate_is_rejected_by_class() {
        // nomos-canon depends on nomos-core, so it reaches rand as well.
        let found = assert_case(
            "rand-dep",
            &[
                PurityCode::DependencyDeniedClass,
                PurityCode::TransitiveDependencyDeniedClass,
            ],
            "nomos-core",
            Some("rand"),
        );
        assert!(found.iter().all(|v| v.class() == Some("randomness")));
        assert!(
            found
                .iter()
                .any(|v| v.package == "nomos-canon" && v.detail.contains("through nomos-core")),
            "{found:#?}"
        );
    }

    #[test]
    fn an_async_runtime_is_rejected_by_class() {
        let found = assert_case(
            "tokio-dep",
            &[PurityCode::DependencyDeniedClass],
            "nomos-canon",
            Some("tokio"),
        );
        assert_eq!(found[0].class(), Some("async-runtime"));
    }

    #[test]
    fn a_crate_in_no_class_is_still_rejected_when_not_allowlisted() {
        assert_case(
            "unlisted-dep",
            &[
                PurityCode::DependencyNotAllowlisted,
                PurityCode::TransitiveDependencyNotAllowlisted,
            ],
            "nomos-core",
            Some("itoa"),
        );
    }

    #[test]
    fn a_build_script_is_rejected() {
        assert_case(
            "build-script",
            &[PurityCode::BuildScript],
            "nomos-core",
            None,
        );
    }

    #[test]
    fn dropping_no_std_is_rejected_even_when_a_doc_comment_mentions_it() {
        assert_case(
            "no-std-dropped",
            &[PurityCode::NoStdMissing],
            "nomos-core",
            None,
        );
    }

    #[test]
    fn an_allowed_dependency_with_default_features_is_rejected() {
        assert_case(
            "default-features",
            &[PurityCode::DependencyFeatures],
            "nomos-canon",
            Some("serde"),
        );
    }

    #[test]
    fn an_allowed_dependency_with_an_unlisted_feature_is_rejected() {
        let found = assert_case(
            "extra-feature",
            &[PurityCode::DependencyFeatures],
            "nomos-canon",
            Some("serde"),
        );
        assert!(found.iter().any(|v| v.detail.contains("std")), "{found:#?}");
    }

    #[test]
    fn a_transitive_dependency_the_policy_does_not_name_is_rejected() {
        // rand is in a denied class, so the class is the reason, checked
        // before the allowlist as it is for a declared dependency.
        let found = assert_case(
            "transitive",
            &[PurityCode::TransitiveDependencyDeniedClass],
            "nomos-canon",
            Some("rand"),
        );
        assert_eq!(found[0].class(), Some("randomness"));
        assert!(found[0].detail.contains("through serde"), "{found:#?}");
    }

    /// Milestone 06 finding: a denied-class crate named in an allowed
    /// dependency's `transitive` list passed the check. The oracle is the
    /// policy's own statement that a denied class is never allowed.
    #[test]
    fn a_transitive_allow_does_not_admit_a_denied_class() {
        let found = assert_case(
            "transitive-denied-class",
            &[
                PurityCode::PolicyAllowsDeniedClass,
                PurityCode::TransitiveDependencyDeniedClass,
            ],
            "nomos-canon",
            Some("rand"),
        );
        assert!(found.iter().all(|v| v.class() == Some("randomness")));
        assert!(
            found
                .iter()
                .any(|v| v.code() == PurityCode::PolicyAllowsDeniedClass
                    && v.package == "serde"
                    && v.dependency() == Some("rand")
                    && v.detail.contains("[allow.serde].transitive")),
            "{found:#?}"
        );
        assert!(
            found
                .iter()
                .any(|v| v.code() == PurityCode::TransitiveDependencyDeniedClass
                    && v.detail.contains("through serde")),
            "{found:#?}"
        );
    }

    #[test]
    fn a_policy_that_allows_a_denied_crate_is_rejected() {
        let found = assert_case(
            "policy-allows-denied-class",
            &[PurityCode::PolicyAllowsDeniedClass],
            "rand",
            Some("rand"),
        );
        assert!(found[0].detail.contains("[allow.rand]"), "{found:#?}");
        // The same allow entry, with serde pulling rand in: the graph walk
        // rejects rand on its own, so the allow entry admits nothing.
        let dir = workspace("allow-denied-reached", Some("transitive"));
        copy_tree(
            &fixtures().join("cases").join("policy-allows-denied-class"),
            &dir,
        );
        let found = check_dir(&dir);
        assert_eq!(
            codes(&found),
            [
                PurityCode::PolicyAllowsDeniedClass,
                PurityCode::TransitiveDependencyDeniedClass,
            ]
            .into(),
            "{found:#?}"
        );
    }

    #[test]
    fn dropping_a_required_lint_is_rejected_per_lint() {
        let found = assert_case(
            "panic-lints-dropped",
            &[PurityCode::RequiredLintMissing],
            "nomos-core",
            None,
        );
        let missing: BTreeSet<&str> = found
            .iter()
            .filter_map(|v| v.detail.split(" deny ").nth(1))
            .map(|s| s.split(' ').next().unwrap())
            .collect();
        assert_eq!(
            missing,
            ["clippy::panic", "clippy::todo", "clippy::unimplemented"].into()
        );
    }

    #[test]
    fn a_manifest_that_does_not_inherit_the_workspace_lints_is_rejected() {
        assert_case(
            "lints-not-workspace",
            &[PurityCode::LintsNotFromWorkspace],
            "nomos-core",
            None,
        );
    }

    #[test]
    fn a_core_crate_the_policy_does_not_list_is_rejected() {
        assert_case(
            "unlisted-core-crate",
            &[PurityCode::CoreCrateUnlisted],
            "nomos-extra",
            None,
        );
    }

    #[test]
    fn a_policy_naming_a_crate_the_workspace_lacks_is_rejected() {
        assert_case(
            "policy-names-unknown-crate",
            &[PurityCode::PolicyNamesUnknownCrate],
            "nomos-ghost",
            None,
        );
    }

    #[test]
    fn a_malformed_policy_is_rejected() {
        assert_case(
            "policy-malformed",
            &[PurityCode::PolicyMalformed],
            "-",
            None,
        );
    }

    #[test]
    fn a_missing_policy_is_rejected_not_skipped() {
        let dir = workspace("policy-missing", None);
        std::fs::remove_file(dir.join(super::POLICY_PATH)).unwrap();
        let found = check_dir(&dir);
        assert_eq!(
            codes(&found),
            [PurityCode::PolicyMissing].into(),
            "{found:#?}"
        );
    }

    #[test]
    fn restoring_the_valid_manifest_makes_the_workspace_pass_again() {
        let dir = workspace("restored", Some("rand-dep"));
        assert!(!check_dir(&dir).is_empty());
        let core = "crates/core/nomos-core/Cargo.toml";
        std::fs::copy(fixtures().join("base").join(core), dir.join(core)).unwrap();
        assert!(check_dir(&dir).is_empty());
    }

    #[test]
    fn inner_attributes_ignore_comments_and_span_lines() {
        let source = "//! #![no_std] in a doc comment\n#![no_std]\n#![deny(\n    clippy::panic, // trailing\n    clippy::todo\n)]\n#![cfg_attr(test, allow(clippy::panic))]\nfn f() {}\n";
        let attrs = inner_attributes(source);
        assert_eq!(
            attrs,
            [
                "no_std",
                "deny( clippy::panic, clippy::todo )",
                "cfg_attr(test, allow(clippy::panic))"
            ]
        );
        assert_eq!(
            denied_lints(&attrs),
            ["clippy::panic", "clippy::todo"].map(str::to_owned).into()
        );
    }

    #[test]
    fn a_lint_denied_only_under_cfg_attr_does_not_count() {
        let attrs = inner_attributes("#![cfg_attr(not(test), deny(clippy::panic))]\n");
        assert!(denied_lints(&attrs).is_empty());
    }

    #[test]
    fn every_case_directory_has_a_test() {
        let cases: BTreeSet<String> = std::fs::read_dir(fixtures().join("cases"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        let source = include_str!("purity.rs");
        for case in &cases {
            assert!(
                source.contains(&format!("\"{case}\"")),
                "fixture case {case} has no test"
            );
        }
        assert_eq!(cases.len(), 16);
    }
}
