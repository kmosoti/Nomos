//! Stable failure codes for snapshot verification.
//!
//! Every failure carries a [`Code`] whose string form is stable. Tests assert
//! on the code, not on message text, so a check that starts failing for a
//! different reason fails its test even though it still fails.

use std::fmt;

macro_rules! codes {
    ($($variant:ident => $text:literal,)*) => {
        /// Why verification failed. The string form is the stable identifier.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum Code {
            $(
                #[doc = $text]
                $variant,
            )*
        }

        impl Code {
            /// The stable identifier, for example `checksum-mismatch`.
            pub(crate) fn as_str(self) -> &'static str {
                match self {
                    $(Code::$variant => $text,)*
                }
            }

            /// Every code, for the uniqueness test.
            #[cfg(test)]
            pub(crate) const ALL: &'static [Code] = &[$(Code::$variant,)*];
        }
    };
}

codes! {
    // Manifest stage.
    ManifestMissing => "manifest-missing",
    ManifestUnreadable => "manifest-unreadable",
    ManifestMalformed => "manifest-malformed",
    ManifestEmpty => "manifest-empty",
    ManifestDuplicateEntry => "manifest-duplicate-entry",
    ManifestNamesMissingFile => "manifest-names-missing-file",
    ChecksumMismatch => "checksum-mismatch",
    // Coverage stage.
    SnapshotUnreadable => "snapshot-unreadable",
    UncoveredFile => "uncovered-file",
    RequiredFileUnlisted => "required-file-unlisted",
    // Strict NDJSON parsing.
    GraphUnreadable => "graph-unreadable",
    NdjsonBom => "ndjson-bom",
    NdjsonNoFinalNewline => "ndjson-no-final-newline",
    NdjsonBlankRecord => "ndjson-blank-record",
    NdjsonNotObject => "ndjson-not-object",
    NdjsonEmpty => "ndjson-empty",
    JsonDuplicateKey => "json-duplicate-key",
    JsonNonFiniteNumber => "json-non-finite-number",
    JsonSyntax => "json-syntax",
    // Graph structure and references.
    MetadataNotFirst => "metadata-not-first",
    MetadataCount => "metadata-count",
    MissingId => "missing-id",
    DuplicateId => "duplicate-id",
    UnknownRecordType => "unknown-record-type",
    MissingField => "missing-field",
    WrongFieldType => "wrong-field-type",
    WeightOutOfRange => "weight-out-of-range",
    MissingRationale => "missing-rationale",
    UnresolvedSource => "unresolved-source",
    SourceWithoutLocator => "source-without-locator",
    RecommendationWithoutExperiment => "recommendation-without-experiment",
    UnresolvedExperiment => "unresolved-experiment",
    MissingAcceptanceCriteria => "missing-acceptance-criteria",
    UnresolvedRecommendation => "unresolved-recommendation",
    ExperimentNotProposal => "experiment-not-proposal",
    CounterexampleIncomplete => "counterexample-incomplete",
    DanglingEndpoint => "dangling-endpoint",
    SelfLoop => "self-loop",
    DanglingEvidence => "dangling-evidence",
    DuplicateEdge => "duplicate-edge",
    DependencyEndpointKind => "dependency-endpoint-kind",
    MissingTestedBy => "missing-tested-by",
    IsolatedNode => "isolated-node",
    DependencyCycle => "dependency-cycle",
    CountsMismatch => "counts-mismatch",
    // JSON Schema stage.
    SchemaUnreadable => "schema-unreadable",
    SchemaInvalid => "schema-invalid",
    SchemaViolation => "schema-violation",
    // Negative controls.
    NegativeControlAccepted => "negative-control-accepted",
    NegativeControlWrongReason => "negative-control-wrong-reason",
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A verification failure: a stable code and a message for people.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerificationError {
    code: Code,
    message: String,
}

impl VerificationError {
    /// A failure with the given code and message.
    pub(crate) fn new(code: Code, message: impl Into<String>) -> Self {
        VerificationError {
            code,
            message: message.into(),
        }
    }

    /// The stable reason.
    pub(crate) fn code(&self) -> Code {
        self.code
    }
}

impl fmt::Display for VerificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for VerificationError {}

impl From<VerificationError> for String {
    fn from(error: VerificationError) -> String {
        error.to_string()
    }
}

/// Result of a verification step.
pub(crate) type Verified<T> = Result<T, VerificationError>;

/// Fails with `code` unless `condition` holds.
pub(crate) fn require(
    condition: bool,
    code: Code,
    message: impl FnOnce() -> String,
) -> Verified<()> {
    if condition {
        Ok(())
    } else {
        Err(VerificationError::new(code, message()))
    }
}

/// Checks that a deliberately corrupted input was rejected, and for the expected reason.
pub(crate) fn expect_rejection<T>(name: &str, result: Verified<T>, expected: Code) -> Verified<()> {
    match result {
        Ok(_) => Err(VerificationError::new(
            Code::NegativeControlAccepted,
            format!("negative control {name} was accepted"),
        )),
        Err(error) if error.code() == expected => Ok(()),
        Err(error) => Err(VerificationError::new(
            Code::NegativeControlWrongReason,
            format!(
                "negative control {name} was rejected as {} instead of {expected}: {error}",
                error.code()
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{Code, VerificationError, expect_rejection};

    #[test]
    fn every_code_has_a_distinct_identifier() {
        let names: BTreeSet<&str> = Code::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(names.len(), Code::ALL.len());
    }

    #[test]
    fn an_accepted_negative_control_is_itself_a_failure() {
        let error = expect_rejection("probe", Ok::<(), VerificationError>(()), Code::DuplicateId)
            .unwrap_err();
        assert_eq!(error.code(), Code::NegativeControlAccepted);
    }

    #[test]
    fn a_negative_control_rejected_for_the_wrong_reason_is_a_failure() {
        let wrong = Err::<(), _>(VerificationError::new(Code::SelfLoop, "x"));
        let error = expect_rejection("probe", wrong, Code::DuplicateId).unwrap_err();
        assert_eq!(error.code(), Code::NegativeControlWrongReason);
    }

    #[test]
    fn the_right_reason_passes() {
        let right = Err::<(), _>(VerificationError::new(Code::DuplicateId, "x"));
        assert!(expect_rejection("probe", right, Code::DuplicateId).is_ok());
    }
}
