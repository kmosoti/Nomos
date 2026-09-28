//! Fixture kernel: an assessment function and a Variance count.

/// The three outcomes of assessing a Condition.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Assessment {
    /// The Observation matches.
    Satisfied,
    /// The Observation differs.
    Variance,
    /// There is no usable Observation.
    Indeterminate,
}

/// Assesses one Condition against an Observation that may have failed.
pub fn assess(expected: u8, observed: Option<u8>) -> Assessment {
    match observed {
        None => Assessment::Indeterminate,
        Some(o) if o == expected => Assessment::Satisfied,
        Some(_) => Assessment::Variance,
    }
}

/// How many Assessments are Variances.
pub fn variances(assessments: &[Assessment]) -> usize {
    assessments
        .iter()
        .filter(|a| **a == Assessment::Variance)
        .count()
}

#[cfg(test)]
mod tests {
    use super::{Assessment, assess, variances};

    #[test]
    fn a_failed_observation_is_indeterminate() {
        assert_eq!(assess(1, None), Assessment::Indeterminate);
    }

    #[test]
    fn a_mismatch_is_a_variance() {
        assert_eq!(assess(1, Some(2)), Assessment::Variance);
    }

    /// Never feeds an Indeterminate, so a count that includes them survives.
    #[test]
    fn only_variances_are_counted() {
        assert_eq!(
            variances(&[Assessment::Satisfied, Assessment::Variance]),
            1
        );
    }
}
