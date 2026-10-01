# Release Process

A release of Nomos is a tag `v<version>` on a commit of `main`. This page says what the repository checks when one is cut, what only the repository host can enforce, and how to cut one. The rules are in [verification-strategy.md](formal/verification-strategy.md#release-admission) and [ADR 0015](adr/0015-generator-verifier-development-model.md).

## The Trust Root

The checks of this repository live in the repository, so they cannot authenticate it. On alpha 1 the trust-boundary job ran on the tag with `origin/main` as its base; the tagged commit was the head of `main`, so the job compared the commit with itself, inspected no commit, and passed. Had a commit with an undeclared verifier change been pushed straight to `main`, tagging it would have produced the same passing result.

Two layers close that.

1. **The repository host decides what `main` and the release tags may accept.** The rules below are applied once, by the owner, in the repository settings. Nothing in the repository can check that they are on, so this page is the record of what they are.
2. **The release job takes the previous release as its base.** It never compares a release with `main`, and it refuses a base that is the release itself.

## The Repository Rules

Two rulesets, as importable JSON, in [`.github/rulesets/`](../.github/rulesets/). In the repository's Settings, under Rules, Rulesets, choose New ruleset, Import a ruleset, and choose each file.

| Ruleset | Target | What it does |
| --- | --- | --- |
| [`main.json`](../.github/rulesets/main.json) | The default branch | Blocks deletion and force pushes. Accepts changes only through a pull request merged with a merge commit, so that a `Trust-Boundary:` declaration survives (AGENTS.md). Requires the checks `check`, `package`, `debian (12)`, `debian (13)`, `debian-minimal (12)`, `debian-minimal (13)`, `trust-boundary`, and `research-frozen` to pass. It has no bypass actor |
| [`release-tags.json`](../.github/rulesets/release-tags.json) | Tags matching `v*` | Blocks creating, moving, and deleting them, except by a repository administrator |

A test (`the_rulesets_require_the_jobs_ci_defines`) fails if a required check names a job the workflow does not define, so the rules and the workflow cannot drift apart unseen.

## What the Release Job Checks

On a push of a tag `v*`, CI runs every job it runs for a commit, and then:

| Check | Refuses when |
| --- | --- |
| `check-release-base`, in `trust-boundary` and `research-frozen` | The previous release tag, which the job finds with `git describe`, does not exist, is the tagged commit, is not an ancestor of it, or the tagged commit is not on `main` |
| `check-trust-boundary --release` and `research frozen`, against that tag | An undeclared oracle change, or a changed research snapshot, since the previous release. A rule the policy dates with `since` is applied only to commits that descend from the commit that added it, because the range can hold commits written before the rule; a pull request and a push to `main` apply every rule to every commit |
| `check-release-chain`, in `release` | A Debian suite is missing or failed, did not install the package being published, or installed a package with a different digest |
| `check-workflow-pins`, in `check` | An action is not pinned to a full commit SHA |

Every one fails closed: a base that cannot be resolved, or a record that cannot be read, fails the job.

## Cutting a Release

1. Merge everything for the release to `main` and wait for its checks to pass.
2. Write `docs/releases/v<version>.md`, the release notes. The release job uses it as the description, and appends the package's digest.
3. Set the workspace version to `<version>` in the root `Cargo.toml`, and merge that too.
4. Tag the commit and push the tag:

   ```sh
   git fetch origin main
   git tag -a v<version> <commit> -m "Nomos <version>"
   git push origin v<version>
   ```

5. Watch the `release` job. It publishes the package the Debian 12 and 13 suites installed, `SHA256SUMS`, `RELEASE-EVIDENCE.json`, and a provenance attestation, as a pre-release.
6. Check what was published:

   ```sh
   sha256sum --check --ignore-missing SHA256SUMS
   gh attestation verify nomos-cell_<version>_amd64.deb --repo kmosoti/Nomos
   ```

A release that turns out to be wrong is not fixed by moving its tag: the tag rules refuse that. Cut the next version.

## Updating the Pinned Actions

Every action in `.github/workflows/` is pinned to a full commit SHA, with its tag in a comment. Dependabot proposes updates weekly ([`.github/dependabot.yml`](../.github/dependabot.yml)). To pin by hand, resolve a tag with `git ls-remote https://github.com/<owner>/<repo>.git refs/tags/<tag>`, using the peeled commit, `refs/tags/<tag>^{}`, when the tag is annotated.
