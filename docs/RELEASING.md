# Releasing

`atlasctl` ships to three places from one pipeline: GitHub Releases (the
`curl | sh` installer), crates.io, and PyPI.

## How a release happens

Merges to `main` do not publish. release-please maintains a standing
"Release vX.Y.Z" pull request, and **merging that PR is the release**. Merging
it tags, builds, and publishes everything since the last one.

That reconciles "merging to main ships" with the fact that crates.io versions
are immutable. Recipe edits — which are this repository's main traffic — land on
`main` immediately and are visible to the website, and they accumulate in the
pending release PR rather than burning a version each.

Commit titles decide the version, so they must be conventional commits:
`feat:` for a minor bump, `fix:` for a patch, `feat!:` or a `BREAKING CHANGE:`
footer for a major. The repository squash-merges, so the PR title becomes the
commit message.

## What publishes, and how it authenticates

| Target | Job | Credential |
|---|---|---|
| GitHub Release (tarballs, checksums, `install.sh`) | `github-release` | the run's own `GITHUB_TOKEN` |
| crates.io | `publish-crates` | Trusted Publishing (OIDC), environment `crates-io` |
| PyPI (`pyatlasctl`) | `publish-pypi` | Trusted Publishing (OIDC), environment `pypi` |

No long-lived registry token is used. Each publish job mints a short-lived one
from its own OIDC identity, and both environments are restricted to `main` and
`v*` tags, so a feature branch cannot reach them.

Both publish steps are idempotent — crates check the index first, PyPI uses
`skip-existing` — so re-running a partially failed release is safe.

## The distribution names

The crate and the binary are both `atlasctl`. The **PyPI distribution is
`pyatlasctl`**: `atlasctl` is taken there by an unrelated project, and PyPI also
refuses names that normalize too close to an existing one, which ruled out
`atlas-ctl`. The wheel's console script keeps the real name, so
`uv tool install pyatlasctl` still puts `atlasctl` on your PATH.

## Recipes are part of the binary

Recipes are compiled in, so **a recipe change only reaches users when a release
ships**. That is the security property, not an oversight: there is no remote
registry to redirect and nothing fetched at runtime. The website reads recipes
from git, so it reflects `main` immediately either way.

The workspace root is itself a package (`atlas-recipes-data`) for this reason —
`cargo package` only includes files beneath the crate root, so a crate under
`crates/` could not embed `../../recipes` and still work when installed from
crates.io.

## If the pipeline is broken

`.github/workflows/bootstrap-publish.yml` publishes to crates.io with a stored
token instead of OIDC. It is manual-dispatch only, dry-runs by default, and
requires a typed confirmation. It exists because the *first* publish of a crate
cannot use Trusted Publishing — a crate must exist before a publisher can be
attached to it — and is kept afterwards only as a recovery path. Delete it, and
revoke `CARGO_REGISTRY_TOKEN`, once `release.yml` has published cleanly at least
once.

## Version history baseline

`v0.1.0` is tagged at the commit the bootstrap workflow published from
(`7b86068`). The tag was added afterwards, because that publish predated the
automated pipeline and did not tag.

This matters: release-please computes the next version from commits **since the
last tag**. Without it, the first release PR proposed 0.2.0 and swept every
historical recipe commit into the changelog, because it had no baseline to
compare against.

## Verifying a release

```sh
cargo install atlasctl --locked      # from crates.io
uvx pyatlasctl list                  # from PyPI, no install step
curl -fsSL https://atlasinference.io/install.sh | sh
```

The installer verifies SHA-256 against the release, and verifies Sigstore build
provenance too when `gh` is present. A `smoke-install` job runs the published
one-liner on both architectures before anyone follows the website's
instructions.
