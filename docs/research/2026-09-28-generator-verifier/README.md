# Generator-Verifier Research Digest, 2026-09-28

A reading of the literature behind one question: when a language model writes the code, what has to be true of the checks for a green result to mean anything? The digest was assembled for the [verification-foundation milestone](../2026-09-28-typed-core/grounding-plan.md#02-verification-foundation) and fed [ADR 0015](../../adr/0015-generator-verifier-development-model.md). It is exploratory. Nothing here binds the implementation; the ADR does.

## What This Digest Is and Is Not

It is a digest, not a snapshot. The [typed-core snapshot](../2026-09-28-typed-core/README.md) is a bundle received as evidence, frozen under a manifest, and never edited. This directory is prose written in this repository from primary sources fetched during the milestone. It may be corrected, and a correction is a commit with the reason in its message.

Every claim below is tagged with one of four categories, and the tag is the important part:

| Tag | Meaning |
| --- | --- |
| **Verified fact** | Read on a fetched primary page: the abstract, the full text, or the author-hosted paper. [sources.md](sources.md) names the page. |
| **Paper-reported result** | A number the paper reports about its own experiment. It is a result about that paper's subjects, models, and benchmarks, not about Nomos. |
| **Architectural inference** | Our reading of what a mechanism implies for a Rust control system. It is an argument, and the experiment that would test it is named. |
| **Open question** | Something the sources do not settle. Listed in [open-questions.md](open-questions.md) with what would settle it. |

A paper-reported result is never quoted as a Nomos result. Several sources were reachable only as abstracts; [sources.md](sources.md) marks those, and nothing beyond their abstract is claimed. The Association for Computing Machinery (ACM) Digital Library and the Institute of Electrical and Electronics Engineers (IEEE) Xplore library refused automated fetches, so venue metadata for a few items comes from conference pages, Crossref, or an author's publication list, and is marked accordingly.

## Files

| File | Contents |
| --- | --- |
| [sources.md](sources.md) | Every source, with title, authors, venue, the page fetched, what it demonstrated, the limitations it states, and its verification status |
| [framework.md](framework.md) | The generator-verifier asymmetry stated precisely, then one card per theme: finding, mechanism, evidence status, transferable lesson, non-transferable assumption, Nomos implication, suggested experiment |
| [open-questions.md](open-questions.md) | What the sources leave open, and the closure rule for each |

## The One-Paragraph Version

A generator, model or human, produces candidates. A verifier judges them against a specification in a pinned environment. The asymmetry that matters is not that generation is hard and verification easy; it is that the generator's output need not be deterministic or even well-formed, while the verifier's verdict on a normalized candidate must be a function of the candidate, the specification, and the environment alone. Every source in this digest is about one of three consequences. First, the generator will satisfy the verifier's letter rather than its intent whenever the two differ, so the specification and the verifier are the assets to protect. Second, a test the generator wrote is not independent evidence about the generator's code, so the oracle has to come from somewhere else: an invariant, a reference implementation, a metamorphic relation, a proof obligation. Third, a green run is a record only when it names what ran, on what, under which bounds, and a check that could not run is not a pass.

## Provenance

Sources were located and fetched during the milestone, on 2026-09-28, by four search agents whose reports were cross-read. Where a report and a fetched page disagreed, the page won. The digest cites arXiv identifiers and author-hosted files because those were readable; the digital-library records they correspond to were not fetched and are not cited as read.
