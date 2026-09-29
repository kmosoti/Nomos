# Candidate Pack: the Assessment Function

You are writing one candidate implementation of the assessment functions of Nomos, a Linux host-state convergence system, from the specification below. Write it as you would in ordinary work, including your own tests.

## What to Deliver

One Rust file, `candidate.rs`, containing:

1. The three public functions below, with exactly these signatures, and any private helpers you need.
2. A `#[cfg(test)] mod tests` with the tests you would write for them, using `use super::*;`.

The file is spliced into the crate's `assessment.rs` module in place of the three functions. That module already has, in scope, the imports and types listed under "Types in Scope": `alloc::vec::Vec`, `crate::condition::{Condition, Content, FileCondition}`, `crate::observation::{Collection, CollectionFailure, FileEvidence, Observation}`, `crate::resource::Digest`, and the enums `Variance`, `Reason`, and `Assessment`. The crate is `no_std` with `alloc`; no `unsafe`, no panics outside tests (no `unwrap`, `expect`, `panic!`, `todo!`, or `unimplemented!` in non-test code). In tests you may use `crate::observation::{Provenance, CollectorId, Window, Instant}` and `crate::resource::ResourcePath` to build Observations, and `alloc::vec!`.

```rust
/// Judges collected evidence against a requirement. Evidence is sufficient
/// by construction here, so the result is Satisfied or a Variance, never
/// Indeterminate.
pub fn assess_evidence(requirement: &FileCondition, evidence: &FileEvidence) -> Assessment {
    todo!()
}

/// Judges one collection outcome: a failure is Indeterminate with its
/// reason, and evidence goes to [`assess_evidence`].
pub fn assess_collection(requirement: &FileCondition, collection: &Collection) -> Assessment {
    todo!()
}

/// Assesses `condition` against every Observation of its resource among
/// `observations`. Observations of other resources are ignored.
pub fn assess(condition: &Condition, observations: &[Observation]) -> Assessment {
    todo!()
}
```

## Specification

### Spec §8, Reconciliation Model

## 8. Reconciliation Model

The central abstraction is a level-triggered reconciliation loop. Kubernetes controllers work this way. Nomos adopts the principle without the Kubernetes object model.

For the Condition $C_r$ on resource $r$:

```text
O_r = observe(r)
A_r = assess(C_r, O_r)            # Satisfied | Variance | Indeterminate
match A_r:
  Satisfied     → nothing to plan for r
  Variance      → plan, apply, verify
  Indeterminate → report the reason; never plan a mutation from it
```

An Indeterminate Assessment of one resource does not hide a Variance of another. The report keeps every Assessment.

Convergence needs more than every Assessment Satisfied. A change can leave an Obligation, a follow-up effect no Condition can observe, and an effect that was started may not yet be Settled. Enforce is done only when every Assessment is Satisfied, every Obligation is discharged, and every relevant effect is Settled ([formal/reconciliation.md](formal/reconciliation.md)).

What actually establishes success? A successful Action means *the intended postcondition was observed*. It does not mean *a command exited with status zero*. Everything else builds on this distinction.


### Formal Definition (reconciliation.md, opening)

# Reconciliation

## Model

For each Condition $C_r$ on a managed resource $r$, core provides (spec §8–§9, [ADR 0005](../adr/0005-assessment-vocabulary.md), [ADR 0006](../adr/0006-kernel-contract.md)):

$$
\begin{aligned}
O_r &= \mathrm{observe}(r) \\
A_r &= \mathrm{assess}(C_r, O_r) \in \{\mathrm{Satisfied},\ \mathrm{Variance}(\delta),\ \mathrm{Indeterminate}(\rho)\}
\end{aligned}
$$

**Soundness requirement on `assess`.** For every resource kind:

$$
\mathrm{assess}(C, O) = \mathrm{Satisfied} \iff O \text{ is sufficient evidence and } O \models C
$$

$$
\mathrm{assess}(C, O) = \mathrm{Variance}(\delta) \iff O \text{ is sufficient evidence and } O \not\models C
$$

Here $O \models C$ means the Observation satisfies the Condition, and *sufficient* means the Observation was collected, is fresh under the policy, and does not contradict another Observation of $r$. Anything else is $\mathrm{Indeterminate}(\rho)$ with the reason $\rho$. Every correctness result below rests on this requirement, so every resource kind tests it directly, including that a denied read never assesses as Satisfied or as Variance.

**Assessments do not aggregate.** A Canon's report is the set $\{A_r\}$. An Indeterminate Assessment of one resource does not erase a Variance of another, and a Variance does not become Indeterminate because an unrelated resource could not be observed (counterexample [`partial-assessment`](../research/2026-09-28-typed-core/README.md)).

**Plan inputs.** Planning is a pure function of explicit inputs:

$$

### ADR 0005, Decision

## Decision

Spec §3 gains six terms and one revised definition. One concept, one name, as before.

| Term | Definition | Why it exists |
| --- | --- | --- |
| Condition | A proposition about one resource that reality is expected to satisfy | The unit a Cell judges. A Canon is Conditions plus relationships |
| Observation | Evidence obtained from Substrate: what was seen, by which source, when, and whether collection succeeded | Provenance and freshness are part of the evidence, not metadata beside it |
| Assessment | The interpretation of a Condition against an Observation: Satisfied, Variance, or Indeterminate | Three outcomes, per Condition, never aggregated into one status for the Canon |
| Variance | A known mismatch | Revised from "difference between desired and observed state" so a failed observation cannot be one |
| Indeterminate | Insufficient or failed evidence, with its reason | Plans no mutation. An Indeterminate Assessment of one resource never erases a Variance of another |
| Obligation | A follow-up effect a completed change requires and no Condition can observe, held durably until discharged | A refresh owed after a file replacement survives a crash only if it is recorded before the replacement |
| Settled | The condition of an effect whose outcome is known and which can cause no further change | Reservation release, retry, and Plan supersession wait for settlement, not for a satisfied Condition |

The algebra, conceptually:

```rust
enum Assessment<V, E> {
    Satisfied,
    Variance(V),
    Indeterminate(E),
}
```

The governing rule: **unknown evidence does not imply noncompliance.** A failed observation is never a Variance.

The rejected synonyms join the list in spec §3: no "Requirement" for Condition, no "Check" or "Evaluation" for Assessment, no "Unknown" for Indeterminate, no "Pending action" for Obligation, no "Done" or "Quiesced" for Settled.


### Invariant N13

| N13 | Unknown evidence does not imply noncompliance | $A_r = \mathrm{Indeterminate}(\rho) \Rightarrow A_r \notin \mathit{Variances} \wedge \forall a \in P: \mathrm{cause}(a) \in \mathit{Variances} \cup \mathit{Obligations}$ | `assess` returns Indeterminate for failed or insufficient evidence; the transition kernel plans an Action only for a Variance, a pending Obligation, or the `on_change` target of an Action, which runs only once that Action's dispatch has recorded its Obligation ([reconciliation](reconciliation.md#model), [ADR 0009](../adr/0009-warp-activation-semantics.md) note) | Truth table, laws 2 and 3 on generated inputs, Kani harness `a_failed_collection_is_indeterminate`, semantic mutants `SM-ASSESS-001` and `SM-ASSESS-002` ([record](../research/2026-09-28-typed-core/results/assessment-algebra.md)). The Obligation clause is argued from the planning rule and not tested on its own |

## Types in Scope

### assessment.rs (the types; the functions are yours)

```rust

use alloc::vec::Vec;

use crate::condition::{Condition, Content, FileCondition};
use crate::observation::{Collection, CollectionFailure, FileEvidence, Observation};
use crate::resource::Digest;

/// A known mismatch between a Condition and sufficient evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Variance {
    /// The Condition requires a file and none exists.
    Missing,
    /// The Condition requires no file and one exists.
    Unexpected,
    /// The file exists with the wrong content.
    ContentDiffers {
        /// The digest the Condition requires.
        expected: Digest,
        /// The digest observed.
        observed: Digest,
    },
}

/// Why evidence was insufficient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Reason {
    /// No Observation of the resource was supplied.
    NoObservation,
    /// Every Observation of the resource failed. When they failed for
    /// different reasons this is the smallest under [`CollectionFailure`]'s
    /// ordering, so that the Assessment does not depend on the order the
    /// Observations arrived in.
    CollectionFailed(CollectionFailure),
    /// Collected Observations of the resource disagree.
    Conflicting,
}

/// The interpretation of a Condition against evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Assessment {
    /// Sufficient evidence, and the Condition holds.
    Satisfied,
    /// Sufficient evidence, and the Condition does not hold.
    Variance(Variance),
    /// Insufficient or failed evidence. Never a mismatch.
    Indeterminate(Reason),
}

impl Assessment {
    /// Whether this is a Variance.
    pub fn is_variance(&self) -> bool {
        matches!(self, Assessment::Variance(_))
    }

    /// Whether this is Indeterminate.
    pub fn is_indeterminate(&self) -> bool {
        matches!(self, Assessment::Indeterminate(_))
    }
}

/// Judges collected evidence against a requirement. Evidence is sufficient
/// by construction here, so the result is Satisfied or a Variance, never
```

### observation.rs

```rust
//! Observations: evidence from Substrate about one resource (spec §3).
//!
//! An Observation says what was seen, by which collector, over which window,
//! and whether collection succeeded. Provenance and the collection outcome
//! are part of the evidence, not metadata beside it (ADR 0005). A collector
//! that could not observe returns an Observation whose [`Collection`]
//! failed, with the reason; it has no channel through which to say
//! "satisfied" or "mismatch" (ADR 0006 §1).
//!
//! Time here is data. An [`Instant`] is a count of nanoseconds on a clock the
//! collector names; the kernel compares instants it is given and never reads
//! a clock (ADR 0016).

use alloc::string::String;
use core::fmt;

use crate::resource::{Digest, ResourcePath};

/// A point on a collector's clock, in nanoseconds. Only comparisons between
/// instants from the same collector are meaningful.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub struct Instant(pub u64);

/// The window over which evidence was collected: it is true of the resource
/// at some moment between `start` and `end`, inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Window {
    start: Instant,
    end: Instant,
}

/// Why two instants do not make a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowError;

impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("window ends before it starts")
    }
}

impl Window {
    /// A window from `start` to `end`; `end` may equal `start`.
    pub fn new(start: Instant, end: Instant) -> Result<Self, WindowError> {
        if end < start {
            return Err(WindowError);
        }
        Ok(Window { start, end })
    }

    /// When collection began.
    pub fn start(&self) -> Instant {
        self.start
    }

    /// When collection ended.
    pub fn end(&self) -> Instant {
        self.end
    }
}

/// Which collector produced an Observation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CollectorId(String);

impl CollectorId {
    /// A collector by name. Any non-empty text; naming is an adapter concern.
    pub fn new(name: &str) -> Option<Self> {
        (!name.is_empty()).then(|| CollectorId(String::from(name)))
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Who collected the evidence and when.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Provenance {
    collector: CollectorId,
    window: Window,
}

impl Provenance {
    /// Evidence from `collector` over `window`.
    pub fn new(collector: CollectorId, window: Window) -> Self {
        Provenance { collector, window }
    }

    /// The collector.
    pub fn collector(&self) -> &CollectorId {
        &self.collector
    }

    /// The collection window.
    pub fn window(&self) -> Window {
        self.window
    }
}

/// What a collector saw of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum FileEvidence {
    /// No file exists at the path.
    Absent,
    /// A file exists with these bytes.
    Present {
        /// The digest of the bytes.
        digest: Digest,
        /// The byte count.
        size: u64,
    },
}

/// Why a collector could not produce evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum CollectionFailure {
    /// The collector was not permitted to read the resource.
    PermissionDenied,
    /// The collector gave up before the resource answered.
    TimedOut,
    /// The resource exists in a form the collector cannot describe, for
    /// example a socket where a file was expected.
    Unsupported,
    /// The operating system reported an error the collector does not classify.
    Io,
}

/// The outcome of one collection attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum Collection {
    /// Evidence was obtained.
    Collected(FileEvidence),
    /// No evidence was obtained, for this reason.
    Failed(CollectionFailure),
}

/// Evidence about one resource, with its provenance.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Observation {
    path: ResourcePath,
    collection: Collection,
    provenance: Provenance,
}

impl Observation {
    /// An Observation of the file at `path`.
    pub fn file(path: ResourcePath, collection: Collection, provenance: Provenance) -> Self {
        Observation {
            path,
            collection,
            provenance,
        }
    }

    /// The resource observed.
    pub fn path(&self) -> &ResourcePath {
        &self.path
    }

    /// What collection produced.
    pub fn collection(&self) -> &Collection {
        &self.collection
    }

    /// Who collected it and when.
    pub fn provenance(&self) -> &Provenance {
        &self.provenance
    }
}

```

### condition.rs

```rust
//! Conditions: propositions reality is expected to satisfy (spec §3).
//!
//! The first resource family is files. A [`FileCondition`] is a sum type, so
//! the contradictions a product type would admit cannot be written: an
//! absent file has no content requirement, because `Absent` has no field for
//! one (ADR 0004 §4, finding `sum-not-product`). A [`Condition`] pairs the
//! requirement with the resource it is about; its fields are private, so a
//! `Condition` exists only through [`Condition::file`], which is the one
//! place validation happens.

use crate::resource::{Digest, ResourcePath};

/// What the bytes of a present file must satisfy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum Content {
    /// Any bytes at all; presence is the whole requirement.
    Any,
    /// Bytes whose digest is exactly this one.
    Exactly(Digest),
}

/// The requirement on one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub enum FileCondition {
    /// No file exists at the path.
    Absent,
    /// A file exists at the path with the given content.
    Present {
        /// The content requirement.
        content: Content,
    },
}

/// One Condition of a Canon: a requirement on one resource.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Condition {
    path: ResourcePath,
    requirement: FileCondition,
}

impl Condition {
    /// A Condition on the file at `path`.
    pub fn file(path: ResourcePath, requirement: FileCondition) -> Self {
        Condition { path, requirement }
    }

    /// The resource the Condition is about.
    pub fn path(&self) -> &ResourcePath {
        &self.path
    }

    /// The requirement on it.
    pub fn requirement(&self) -> &FileCondition {
        &self.requirement
    }
}
```

### resource.rs

```rust
//! Resource identity for the file family: a validated absolute path and a
//! content digest.
//!
//! Both types have private fields and fallible constructors. A `ResourcePath`
//! that exists is absolute, has no empty, `.`, or `..` component, and has no
//! trailing separator except for the root itself. A `Digest` is exactly 32
//! bytes. Nothing else can be built, so nothing downstream checks again
//! (ADR 0004 §4).

use alloc::string::String;
use core::fmt;

/// Why a string is not a resource path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathError {
    /// The path is empty.
    Empty,
    /// The path does not start with `/`.
    Relative,
    /// A component is empty (`//`), `.`, or `..`.
    UnnormalizedComponent,
    /// A path other than `/` ends with `/`.
    TrailingSeparator,
    /// The path contains a NUL byte, which no operating system path may.
    Nul,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PathError::Empty => "path is empty",
            PathError::Relative => "path is not absolute",
            PathError::UnnormalizedComponent => "path has an empty, `.`, or `..` component",
            PathError::TrailingSeparator => "path ends with a separator",
            PathError::Nul => "path contains a NUL byte",
        })
    }
}

/// An absolute, normalized path naming one file resource on a host.
///
/// Equality is textual. Two paths that name the same inode through a symlink
/// are different resources here; resolution happens at the Substrate
/// boundary, at the moment of use (ADR 0013 §2).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourcePath(String);

impl ResourcePath {
    /// Validates `text` as an absolute, normalized path.
    pub fn new(text: &str) -> Result<Self, PathError> {
        if text.is_empty() {
            return Err(PathError::Empty);
        }
        if text.contains('\0') {
            return Err(PathError::Nul);
        }
        let Some(rest) = text.strip_prefix('/') else {
            return Err(PathError::Relative);
        };
        if rest.is_empty() {
            return Ok(ResourcePath(String::from("/")));
        }
        if rest.ends_with('/') {
            return Err(PathError::TrailingSeparator);
        }
        if rest
            .split('/')
            .any(|component| matches!(component, "" | "." | ".."))
        {
            return Err(PathError::UnnormalizedComponent);
        }
        Ok(ResourcePath(String::from(text)))
    }

    /// The path as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// An unvalidated, empty path for the bounded-verifier harnesses: it
    /// allocates nothing, which keeps the harnesses over enums that carry a
    /// path tractable. It exists only under `cfg(kani)`.
    #[cfg(kani)]
    pub(crate) fn for_harness() -> Self {
        ResourcePath(String::new())
    }
}

impl fmt::Debug for ResourcePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ResourcePath({:?})", self.0)
    }
}

impl fmt::Display for ResourcePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why a value is not a digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestError {
    /// The hex text is not exactly 64 characters.
    Length,
    /// A character is not a hex digit.
    NotHex,
}

impl fmt::Display for DigestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DigestError::Length => "digest is not 64 hex characters",
            DigestError::NotHex => "digest has a non-hex character",
        })
    }
}

/// A 256-bit content digest. Which hash function produced it is fixed by the
/// Canon that names it; this type only carries the bytes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub struct Digest([u8; 32]);

impl Digest {
    /// A digest from its bytes.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Digest(bytes)
    }

    /// A digest from 64 lowercase or uppercase hex characters.
    pub fn from_hex(text: &str) -> Result<Self, DigestError> {
        if text.len() != 64 {
            return Err(DigestError::Length);
        }
        let mut bytes = [0u8; 32];
        for (i, pair) in text.as_bytes().chunks(2).enumerate() {
            let hi = hex_value(pair[0]).ok_or(DigestError::NotHex)?;
            let lo = hex_value(pair[1]).ok_or(DigestError::NotHex)?;
            if let Some(slot) = bytes.get_mut(i) {
                *slot = (hi << 4) | lo;
            }
        }
        Ok(Digest(bytes))
    }

    /// The digest bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

fn hex_value(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Digest(")?;
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        f.write_str(")")
    }
}

```
