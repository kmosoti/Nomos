# Prose Style

Nomos prose follows the Kennedy prose specification: [prose-spec.yaml](prose-spec.yaml). Direct, compact, skeptical, first-principles. Mechanisms over labels. Dry humor, rarely, and never in place of an argument.

Docs use the spec's *polished* mode. Chat, issues, and review threads can use *conversational* mode.

## Short Form

<!-- vale off -->

Write like Kennedy: direct, compact, technically curious, skeptical, and first-principles oriented.

Start with substance, not praise or introductory filler. Prefer mechanisms, contracts, invariants, state ownership, causal structure, and failure behavior over product descriptions or feature lists.

When discussing technologies, descend beneath the named abstraction and explain what actually performs the work. Separate primitive from implementation, mechanism from policy, and independent tradeoff dimensions from an overall "better/worse" judgment.

Challenge assumptions when evidence warrants it, but do not manufacture contrarianism. Clearly distinguish established facts, evidence-supported conclusions, deductions, plausible inferences, and speculation.

Prefer concrete nouns and strong verbs. Use short-to-medium sentences and occasional probing questions. Explain unfamiliar jargon and acronyms on first use.

Avoid corporate prose, hype, generic praise, canned transitions, empty adjectives such as "robust" or "scalable" without specifying why, and generic closing invitations.

Explore unconventional possibilities when technically plausible, but identify the mechanism and convert the hunch into a testable proposition.

Use correct spelling and grammar. Do not imitate the user's keyboard mistakes. Preserve the structure of the thinking, not transcription noise.

<!-- vale on -->

## Tooling

[Vale](https://vale.sh) enforces the rules a regular expression can catch. The rules live in [`.vale/styles/Kennedy/`](../../.vale/styles/Kennedy/), one file per rule ID.

```sh
.vale/lint.sh
```

The script lints every tracked Markdown file, then runs the spec's acceptance tests. Every paragraph in [`.vale/fixtures/reject.md`](../../.vale/fixtures/reject.md) must raise an alert, and [`.vale/fixtures/accept.md`](../../.vale/fixtures/accept.md) must raise none. If a rule change lets marketing copy through, the script fails.

## Rule Coverage

A linter can catch vocabulary. It cannot tell whether a paragraph has a mechanism in it. So the rules split in two.

<!-- The table quotes the phrases the rules ban, so the linter skips it. -->
<!-- vale off -->

| Rule | Checked by | Notes |
| --- | --- | --- |
| KEN001 praise openers | Vale, error | Paragraph-initial praise or validation |
| KEN002 throat-clearing | Vale, warning | Canned introductions and transitions |
| KEN003 corporate filler | Vale, error | Includes the spec's bad headings and discouraged phrasing |
| KEN005 unqualified evaluation | Vale, warning | "Faster", "robust", "scalable", and friends |
| KEN006 speculation as fact | Vale, error | Heuristic: flags certainty words ("obviously", "proves that", "will always") |
| KEN007 unexpanded acronyms | Vale, warning | Order-aware; the audience-known list is in `KEN007.yml` |
| KEN009 popularity arguments | Vale, warning | "Industry standard", "widely adopted", "modern" |
| KEN017 recap sections | Vale, warning | Headings such as Summary and Conclusion |
| KEN018 closing invitations | Vale, warning | "Let me know", "feel free to" |
| KEN004 recommendation without mechanism | Review | |
| KEN008 architecture without contract | Review | |
| KEN010 product vs. concept | Review | |
| KEN011 facts without synthesis | Review | |
| KEN012 everything is a bullet | Review | |
| KEN014 lowest useful primitive | Review | |
| KEN015 failure condition | Review | |
| KEN016 premises exposed | Review | |
| KEN013 typo emulation | Not applicable | Nomos docs never emulate raw chat |

<!-- vale on -->

A warning can be kept when fixing it would damage clarity (spec `pass_6_lint`). Silence a deliberate exception locally with `<!-- vale Kennedy.KEN005 = NO -->` and `<!-- vale Kennedy.KEN005 = YES -->`, and say why in the surrounding prose.

## Mechanics

The spec covers voice. These cover formatting:

- The README tells the story. Technical material lives in `docs/`.
- American spelling: behavior, organize, artifact
- Title Case headings, short and technical, as in the spec's examples ("Failure Model", "Why SQLite?")
- Oxford comma
- One paragraph per line, no hard wrapping
- Bold lead-ins end with a period: `- **Label.** Text.`
- "For example", not "e.g."
- GitHub math syntax (`$...$`, `$$...$$`)
- Mermaid diagrams only, with no labels on arrows or state transitions (see [CONTRIBUTING.md](../../CONTRIBUTING.md#diagrams))

[markdownlint](https://github.com/DavidAnson/markdownlint-cli2) checks Markdown structure against `.markdownlint-cli2.jsonc`:

```sh
npx markdownlint-cli2 "**/*.md"
```
