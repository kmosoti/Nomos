//! Verification receipts (ADR 0007 §1, ADR 0015): a claim needs a record, and
//! a record needs a run.
//!
//! A receipt is one line of NDJSON under `verification/receipts/`, describing
//! one executed check: which registered check, the exact command, the commit
//! and toolchain it ran on, the exit status and output digests, and what the
//! result does not cover. `receipts record <check-id> -- <command...>` runs
//! the command and writes the line from what happened; it cannot write
//! `passed` for a command that did not exit 0, because it never asks. The
//! validator, `receipts validate`, holds every line to
//! `verification/receipt.schema.json` and to `verification/checks.toml`: a
//! `passed` without evidence of a zero exit, a `not-run` with evidence, an
//! unregistered check, or a command that is not the registered one is
//! rejected with a stable [`ReceiptCode`].
//!
//! A receipt is not a proof and carries no signature. It is a record that a
//! named command ran on a named tree and what it returned, so that a status
//! in the verification matrix can be traced to a run. Whoever can edit the
//! repository can write a receipt by hand; the validator makes a false one
//! costly to write, not impossible.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::manifest::sha256_hex;
use crate::strict_json;

/// The receipt schema, relative to the repository root.
pub(crate) const SCHEMA_PATH: &str = "verification/receipt.schema.json";
/// The check registry, relative to the repository root.
pub(crate) const REGISTRY_PATH: &str = "verification/checks.toml";
/// Where receipts live, relative to the repository root.
pub(crate) const RECEIPTS_DIR: &str = "verification/receipts";

/// Why a receipt is rejected. The kebab-case form is the stable identifier.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ReceiptCode {
    /// The registry or the schema is missing or does not parse.
    RegistryUnusable,
    /// A receipt file cannot be read.
    ReceiptUnreadable,
    /// A line is not one strict JSON object.
    ReceiptMalformed,
    /// A line fails the JSON Schema.
    ReceiptSchemaViolation,
    /// `result` is `passed` without evidence of a zero exit status.
    PassedWithoutEvidence,
    /// `result` says one thing and the exit status another.
    ResultContradictsExitStatus,
    /// `result` is `not-run` or `not-applicable` but evidence is present.
    NotRunWithEvidence,
    /// `check_id` is not in the registry.
    UnknownCheck,
    /// `command` does not begin with the registered command.
    CommandMismatch,
}

impl ReceiptCode {
    /// The stable identifier.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ReceiptCode::RegistryUnusable => "receipt-registry-unusable",
            ReceiptCode::ReceiptUnreadable => "receipt-unreadable",
            ReceiptCode::ReceiptMalformed => "receipt-malformed",
            ReceiptCode::ReceiptSchemaViolation => "receipt-schema-violation",
            ReceiptCode::PassedWithoutEvidence => "receipt-passed-without-evidence",
            ReceiptCode::ResultContradictsExitStatus => "receipt-result-contradicts-exit-status",
            ReceiptCode::NotRunWithEvidence => "receipt-not-run-with-evidence",
            ReceiptCode::UnknownCheck => "receipt-unknown-check",
            ReceiptCode::CommandMismatch => "receipt-command-mismatch",
        }
    }
}

/// One rejected line.
#[derive(Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ReceiptViolation {
    code: ReceiptCode,
    file: String,
    line: usize,
    detail: String,
}

impl ReceiptViolation {
    /// The stable reason.
    #[cfg(test)]
    pub(crate) fn code(&self) -> ReceiptCode {
        self.code
    }
}

impl fmt::Display for ReceiptViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {}:{}: {}",
            self.code.as_str(),
            self.file,
            self.line,
            self.detail
        )
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    checks: BTreeMap<String, Check>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Check {
    command: String,
    #[allow(dead_code, reason = "read by people; the registry requires it")]
    what: String,
}

impl Check {
    /// The registered command up to its first `<placeholder>`.
    fn prefix(&self) -> &str {
        self.command
            .split_once('<')
            .map_or(self.command.as_str(), |(head, _)| head.trim_end())
    }
}

/// The validator's report.
#[derive(Serialize)]
pub(crate) struct Report {
    files: Vec<String>,
    receipts: usize,
    violations: Vec<ReceiptViolation>,
}

impl Report {
    /// The rejected lines, in file and line order.
    pub(crate) fn violations(&self) -> &[ReceiptViolation] {
        &self.violations
    }

    /// How many lines were accepted or rejected.
    pub(crate) fn receipts(&self) -> usize {
        self.receipts
    }
}

/// Where the validator finds its inputs.
pub(crate) struct Paths {
    /// The JSON Schema.
    pub(crate) schema: PathBuf,
    /// The check registry.
    pub(crate) registry: PathBuf,
    /// The directory of `*.ndjson` receipt files.
    pub(crate) receipts: PathBuf,
}

impl Paths {
    /// The repository's own layout under `root`.
    pub(crate) fn under(root: &Path) -> Paths {
        Paths {
            schema: root.join(SCHEMA_PATH),
            registry: root.join(REGISTRY_PATH),
            receipts: root.join(RECEIPTS_DIR),
        }
    }
}

fn unusable(path: &Path, detail: impl fmt::Display) -> ReceiptViolation {
    ReceiptViolation {
        code: ReceiptCode::RegistryUnusable,
        file: path.display().to_string(),
        line: 0,
        detail: detail.to_string(),
    }
}

/// Validates every `*.ndjson` file under `paths.receipts`.
pub(crate) fn validate(paths: &Paths) -> Result<Report, String> {
    let mut violations = Vec::new();
    let registry: Option<Registry> = match std::fs::read_to_string(&paths.registry) {
        Ok(text) => match toml::from_str(&text) {
            Ok(registry) => Some(registry),
            Err(e) => {
                violations.push(unusable(&paths.registry, e.message()));
                None
            }
        },
        Err(e) => {
            violations.push(unusable(&paths.registry, e));
            None
        }
    };
    let validator = std::fs::read(&paths.schema)
        .map_err(|e| e.to_string())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).map_err(|e| e.to_string()))
        .and_then(|schema| {
            jsonschema::meta::validate(&schema).map_err(|e| e.to_string())?;
            jsonschema::options()
                .should_validate_formats(true)
                .build(&schema)
                .map_err(|e| e.to_string())
        });
    let validator = match validator {
        Ok(validator) => Some(validator),
        Err(e) => {
            violations.push(unusable(&paths.schema, e));
            None
        }
    };

    let mut files: Vec<PathBuf> = match std::fs::read_dir(&paths.receipts) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "ndjson"))
            .collect(),
        Err(e) => return Err(format!("{}: {e}", paths.receipts.display())),
    };
    files.sort();
    let mut receipts = 0usize;
    for file in &files {
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(e) => {
                violations.push(ReceiptViolation {
                    code: ReceiptCode::ReceiptUnreadable,
                    file: name,
                    line: 0,
                    detail: e.to_string(),
                });
                continue;
            }
        };
        for (index, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            receipts += 1;
            let mut reject = |code, detail: String| {
                violations.push(ReceiptViolation {
                    code,
                    file: name.clone(),
                    line: index + 1,
                    detail,
                });
            };
            let record = match strict_json::parse(line) {
                Ok(Value::Object(map)) => Value::Object(map),
                Ok(_) => {
                    reject(ReceiptCode::ReceiptMalformed, "not a JSON object".into());
                    continue;
                }
                Err(e) => {
                    reject(ReceiptCode::ReceiptMalformed, e.to_string());
                    continue;
                }
            };
            if let Some(validator) = &validator {
                let errors: Vec<String> = validator
                    .iter_errors(&record)
                    .map(|e| format!("{} (schema path {})", e, e.schema_path()))
                    .collect();
                if !errors.is_empty() {
                    reject(ReceiptCode::ReceiptSchemaViolation, errors.join("; "));
                }
            }
            // The semantic rules, stated again in code so that a weakened
            // schema does not weaken the validator.
            let result = record["result"].as_str().unwrap_or("");
            let exit = record["evidence"]["exit_status"].as_i64();
            match (result, exit) {
                ("passed", None) => reject(
                    ReceiptCode::PassedWithoutEvidence,
                    "result is passed but there is no evidence of a run".into(),
                ),
                ("passed", Some(status)) if status != 0 => reject(
                    ReceiptCode::ResultContradictsExitStatus,
                    format!("result is passed but the exit status is {status}"),
                ),
                ("failed", Some(0)) => reject(
                    ReceiptCode::ResultContradictsExitStatus,
                    "result is failed but the exit status is 0".into(),
                ),
                ("not-run" | "not-applicable", Some(_)) => reject(
                    ReceiptCode::NotRunWithEvidence,
                    format!("result is {result} but evidence is present"),
                ),
                _ => {}
            }
            if let Some(registry) = &registry {
                let id = record["check_id"].as_str().unwrap_or("");
                match registry.checks.get(id) {
                    None => reject(
                        ReceiptCode::UnknownCheck,
                        format!("check_id {id:?} is not in {REGISTRY_PATH}"),
                    ),
                    Some(check) => {
                        let command = record["command"].as_str().unwrap_or("");
                        if !command.starts_with(check.prefix()) {
                            reject(
                                ReceiptCode::CommandMismatch,
                                format!(
                                    "command {command:?} does not begin with the registered {:?}",
                                    check.prefix()
                                ),
                            );
                        }
                    }
                }
            }
        }
    }
    violations.sort();
    Ok(Report {
        files: files
            .iter()
            .map(|f| {
                f.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect(),
        receipts,
        violations,
    })
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Tests reported by `cargo test`: the sums of `N passed` and `N failed`
/// over every `test result:` line. `None` when there is no such line.
pub(crate) fn test_counts(stdout: &str) -> Option<(u64, u64)> {
    let mut found = false;
    let (mut passed, mut failed) = (0u64, 0u64);
    for line in stdout.lines().filter(|l| l.starts_with("test result:")) {
        found = true;
        let words: Vec<&str> = line.split_whitespace().collect();
        for pair in words.windows(2) {
            let count = pair[0].parse::<u64>().unwrap_or(0);
            match pair[1].trim_end_matches(';') {
                "passed" => passed += count,
                "failed" => failed += count,
                _ => {}
            }
        }
    }
    found.then_some((passed, failed))
}

/// What `record` needs to know about the tree it runs in.
pub(crate) struct Context {
    /// The repository root.
    pub(crate) root: PathBuf,
    /// The check to run, a key of the registry.
    pub(crate) check_id: String,
    /// The command line, `argv[0]` first; run directly, never through a shell.
    pub(crate) argv: Vec<String>,
    /// Properties the check bears on.
    pub(crate) properties: Vec<String>,
    /// What the result does not cover.
    pub(crate) unchecked: String,
}

/// Runs the command and returns the receipt line, without writing it.
pub(crate) fn record(ctx: &Context) -> Result<Value, String> {
    let registry: Registry = toml::from_str(
        &std::fs::read_to_string(ctx.root.join(REGISTRY_PATH))
            .map_err(|e| format!("{REGISTRY_PATH}: {e}"))?,
    )
    .map_err(|e| format!("{REGISTRY_PATH}: {}", e.message()))?;
    let check = registry
        .checks
        .get(&ctx.check_id)
        .ok_or_else(|| format!("{}: not a check in {REGISTRY_PATH}", ctx.check_id))?;
    let command_line = ctx.argv.join(" ");
    if !command_line.starts_with(check.prefix()) {
        return Err(format!(
            "{:?} does not begin with the registered command {:?}",
            command_line,
            check.prefix()
        ));
    }
    let (program, args) = ctx
        .argv
        .split_first()
        .ok_or("a command is required after --")?;
    let commit = git(&ctx.root, &["rev-parse", "HEAD"])?.trim().to_owned();
    let dirty = !git(&ctx.root, &["status", "--porcelain"])?
        .trim()
        .is_empty();
    let toolchain: toml::Table = toml::from_str(
        &std::fs::read_to_string(ctx.root.join("rust-toolchain.toml"))
            .map_err(|e| format!("rust-toolchain.toml: {e}"))?,
    )
    .map_err(|e| format!("rust-toolchain.toml: {}", e.message()))?;
    let channel = toolchain
        .get("toolchain")
        .and_then(|t| t.get("channel"))
        .and_then(toml::Value::as_str)
        .ok_or("rust-toolchain.toml has no toolchain.channel")?
        .to_owned();
    let lockfile =
        std::fs::read(ctx.root.join("Cargo.lock")).map_err(|e| format!("Cargo.lock: {e}"))?;

    let started = Instant::now();
    let out = Command::new(program)
        .args(args)
        .current_dir(&ctx.root)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let exit_status = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut evidence = json!({
        "exit_status": exit_status,
        "stdout_sha256": sha256_hex(&out.stdout),
        "stderr_sha256": sha256_hex(&out.stderr),
        "duration_ms": duration_ms,
    });
    if let Some((passed, failed)) = test_counts(&stdout) {
        evidence["tests_run"] = json!(passed + failed);
        evidence["tests_failed"] = json!(failed);
    }
    let result = if exit_status == 0 { "passed" } else { "failed" };
    let mut receipt = json!({
        "check_id": ctx.check_id,
        "result": result,
        "command": command_line,
        "commit": commit,
        "dirty": dirty,
        "toolchain": channel,
        "lockfile_sha256": sha256_hex(&lockfile),
        "recorded_by": "cargo xtask receipts record",
        "unchecked": ctx.unchecked,
        "evidence": evidence,
    });
    if !ctx.properties.is_empty() {
        receipt["properties"] = json!(ctx.properties);
    }
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    use super::{Paths, ReceiptCode, ReceiptViolation, test_counts, validate};

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }

    fn fixtures() -> PathBuf {
        root().join("tests/fixtures/receipts")
    }

    /// The real schema and registry over one fixture directory of receipts.
    fn run(dir: &Path) -> Vec<ReceiptViolation> {
        validate(&Paths {
            schema: root().join(super::SCHEMA_PATH),
            registry: root().join(super::REGISTRY_PATH),
            receipts: dir.to_path_buf(),
        })
        .unwrap()
        .violations()
        .to_vec()
    }

    fn codes(found: &[ReceiptViolation]) -> BTreeSet<ReceiptCode> {
        found.iter().map(ReceiptViolation::code).collect()
    }

    fn assert_case(case: &str, expected: &[ReceiptCode]) {
        let found = run(&fixtures().join("cases").join(case));
        assert_eq!(
            codes(&found),
            expected.iter().copied().collect(),
            "{case}: {found:#?}"
        );
    }

    #[test]
    fn the_repository_receipts_validate() {
        let report = validate(&Paths::under(&root())).unwrap();
        assert!(report.violations().is_empty(), "{:#?}", report.violations());
    }

    #[test]
    fn a_valid_receipt_passes_so_the_validator_cannot_pass_by_rejecting_everything() {
        let report = validate(&Paths {
            schema: root().join(super::SCHEMA_PATH),
            registry: root().join(super::REGISTRY_PATH),
            receipts: fixtures().join("valid"),
        })
        .unwrap();
        assert!(report.violations().is_empty(), "{:#?}", report.violations());
        assert_eq!(report.receipts(), 3);
    }

    #[test]
    fn passed_without_evidence_is_rejected_by_schema_and_by_rule() {
        assert_case(
            "passed-without-evidence",
            &[
                ReceiptCode::ReceiptSchemaViolation,
                ReceiptCode::PassedWithoutEvidence,
            ],
        );
    }

    #[test]
    fn passed_with_a_nonzero_exit_status_is_rejected() {
        assert_case(
            "passed-nonzero-exit",
            &[
                ReceiptCode::ReceiptSchemaViolation,
                ReceiptCode::ResultContradictsExitStatus,
            ],
        );
    }

    #[test]
    fn failed_with_a_zero_exit_status_is_rejected() {
        assert_case(
            "failed-zero-exit",
            &[
                ReceiptCode::ReceiptSchemaViolation,
                ReceiptCode::ResultContradictsExitStatus,
            ],
        );
    }

    #[test]
    fn not_run_with_evidence_is_rejected() {
        assert_case(
            "not-run-with-evidence",
            &[
                ReceiptCode::ReceiptSchemaViolation,
                ReceiptCode::NotRunWithEvidence,
            ],
        );
    }

    #[test]
    fn an_unregistered_check_is_rejected() {
        assert_case("unknown-check", &[ReceiptCode::UnknownCheck]);
    }

    #[test]
    fn a_command_other_than_the_registered_one_is_rejected() {
        assert_case("command-mismatch", &[ReceiptCode::CommandMismatch]);
    }

    #[test]
    fn a_line_that_is_not_strict_json_is_rejected() {
        assert_case("malformed", &[ReceiptCode::ReceiptMalformed]);
    }

    #[test]
    fn a_missing_required_field_is_a_schema_violation() {
        assert_case("missing-unchecked", &[ReceiptCode::ReceiptSchemaViolation]);
    }

    #[test]
    fn a_missing_registry_rejects_every_receipt_instead_of_skipping_the_check() {
        let report = validate(&Paths {
            schema: root().join(super::SCHEMA_PATH),
            registry: fixtures().join("no-such-registry.toml"),
            receipts: fixtures().join("valid"),
        })
        .unwrap();
        assert_eq!(
            codes(report.violations()),
            [ReceiptCode::RegistryUnusable].into()
        );
    }

    #[test]
    fn test_counts_sum_every_result_line() {
        let out = "test result: ok. 3 passed; 0 failed; 1 ignored\nnoise\ntest result: FAILED. 2 passed; 1 failed; 0 ignored\n";
        assert_eq!(test_counts(out), Some((5, 1)));
        assert_eq!(test_counts("no tests here"), None);
    }

    #[test]
    fn every_case_directory_has_a_test() {
        let cases: BTreeSet<String> = std::fs::read_dir(fixtures().join("cases"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        let source = include_str!("receipt.rs");
        for case in &cases {
            assert!(
                source.contains(&format!("\"{case}\"")),
                "fixture case {case} has no test"
            );
        }
        assert_eq!(cases.len(), 8);
    }
}
