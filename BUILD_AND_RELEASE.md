# Build and Release for Brume

Internal reference for cutting a Brume release. For the *dev-loop and
deploy* workflow (build, sync to a CM5, restart the running service,
troubleshoot bring-up), see `DEPLOY.md`. For the *user-facing* install
path on a fresh CM5, see `INSTALL.md`. Once the public-facing
`RELEASING.md` lands, this doc is its internal companion: how the
release procedure actually works, where every version string comes
from, and what to do when one of the pieces fails.

This doc tracks the current state of the procedure. Items marked
*pending* are described against the planned shape but not yet wired up.

## 1. Versioning

`MAJOR.MINOR.PATCH`, [SemVer 2.0](https://semver.org/spec/v2.0.0.html).

Pre-1.0:

- `0.X.0`: anything user-observable, including new engine features,
  Lua API additions, control surface support, and meaningful UI
  reshuffles.
- `0.X.Y`: bug fixes only. No user-observable behavior change except
  "the broken thing now works."

Post-1.0:

- `MAJOR`: breaking change to the Lua script API (user scripts break)
  or to the Meridian wire format (USB descriptor or channel layout
  change visible to a host DAW).
- `MINOR`: additive Lua API, new engines, new UI pages.
- `PATCH`: bug fixes.

The rationale for SemVer over CalVer is that Brume has a real
compatibility contract with user Lua scripts. CalVer's `2026.04.22`
timestamp doesn't communicate whether a user's scripts will still run.

The current version is `0.3.0` — the first public release. The release
procedure documented here was established at the earlier `0.1.0 → 0.2.0`
step, the transition from "no release procedure" to "an actual cuttable
release".

## 2. Single workspace version

The root `Cargo.toml` is the single source of truth:

```toml
[workspace.package]
version = "0.3.0"
```

Every member crate inherits from it:

```toml
[package]
name = "brume-*"
version.workspace = true
```

Bumping the root field is the whole version change on the code side.
Per-crate SemVer independence is explicitly out of scope: every crate
in this workspace ships as one artifact family under one version.
`cargo build --workspace` after a bump should be the only validation
needed for the version itself; in-tree code that calls
`env!("CARGO_PKG_VERSION")` picks up the new value at compile time
without further plumbing.

## 3. Stamping surfaces

| Where | What stamps it | How |
|---|---|---|
| Workspace `Cargo.toml` | source of truth | edited manually, or by `scripts/release.sh` once that lands |
| Every crate `Cargo.toml` | inherits from workspace | `version.workspace = true` |
| `brumectl --version` | compile-time | clap reads `CARGO_PKG_VERSION` automatically |
| In-app readouts (SYS page, footer) | compile-time | `env!("CARGO_PKG_VERSION")` at each display site |
| Git tag | per release | `v0.3.0` (the `v` prefix matches GitHub Release convention) |
| `CHANGELOG.md` `[X.Y.Z]` header | per release | release script writes the header + date stamp |
| GitHub Release title | per release | `Brume 0.3.0` |
| Release artifact names | per release | e.g. `brume-0.3.0-aarch64-unknown-linux-gnu` |
| `brume-web` source rev strings | per release | `release.yml`'s `stamp-web` job fires a `repository_dispatch` at `aftertonesignal/brume-web`; that repo's `stamp.yml` workflow rewrites the contents of every `<!--BRUME-VERSION-->...<!--/BRUME-VERSION-->` marker pair in top-level HTML files and commits the bump back to its main branch |

The cross-repo brume-web source stamp requires a fine-grained PAT
exposed to the brume repo as the `BRUME_WEB_DISPATCH_TOKEN` secret.
Scope: `aftertonesignal/brume-web` only; permission: `Actions: read
and write`. Without that secret the `stamp-web` job emits a warning
and exits cleanly — the GitHub Release is already published, and the
brume-web source keeps its previous version literal until the next
manual stamp.

`stamp.yml` only modifies source. It does **not** deploy. The
maintainer runs `flyctl deploy` from `~/code/aftertone/brume-web/`
on their own cadence; the live site reflects whatever literal the
source currently has. Ad-hoc stamps without cutting a brume release
are a one-click `workflow_dispatch` on brume-web's `stamp.yml`
(input `version` is required; default sentinel for unreleased
states is `dev`).

## 4. CHANGELOG discipline

`CHANGELOG.md` lives at the repo root, in [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/)
format. Per-release sections use the standard headings: **Added**,
**Changed**, **Fixed**, **Removed**, **Deprecated**, **Security**.

The `[Unreleased]` block sits at the top of the file during the cycle.
Each PR or landing that changes user-observable behavior adds a bullet
to the appropriate sub-section under `[Unreleased]`. The release
procedure promotes that block to a versioned header on cut day.

CHANGELOG entries are not auto-generated from commit messages. Commit
messages are tactical (pair-programming notes); CHANGELOG entries are
strategic (what users see). Writing them by hand forces the
release-cutter to think about user impact, and produces output that
reads like product notes rather than a git diff.

## 5. Cutting a release

The procedure has three phases. Phase 1 is automated by
`scripts/release.sh` once that lands; today it is manual. Phases 2 and
3 are automated by GitHub Actions on tag push once the release
workflow lands; today they are manual.

### 5a. Phase 1 (local): bump, changelog, tag

1. Verify a clean working tree on `main`.
2. Decide the new version per §1.
3. Edit `Cargo.toml`'s `[workspace.package] version` field.
4. Promote the `[Unreleased]` block in `CHANGELOG.md` to a `[X.Y.Z] -
   YYYY-MM-DD` header; reseed an empty `[Unreleased]` above it.
5. Commit the bump and the changelog edit as one commit:
   `release: cut X.Y.Z`.
6. Tag: `git tag vX.Y.Z` (lightweight) or `git tag -s vX.Y.Z` (signed,
   when signing infrastructure is in place; see §7).
7. Push: `git push --follow-tags`.

`scripts/release.sh` (*pending*) wraps steps 3-6, accepts a target
version, opens `$EDITOR` on the changelog, and refuses to run on a
dirty tree or off `main`. It is dry-run by default; pass `--apply` to
mutate state.

### 5b. Phase 2 (CI): build matrix, attach artifacts

`.github/workflows/release.yml` (*pending*) triggers on tag push
matching `v*`. It runs a four-target build matrix:

| Triple | Runner | Artifact |
|---|---|---|
| `aarch64-apple-darwin` | macOS arm64 | `brumectl-X.Y.Z-macos-arm64` |
| `x86_64-apple-darwin` | macOS x86_64 | `brumectl-X.Y.Z-macos-x86_64` |
| `x86_64-unknown-linux-gnu` | ubuntu | `brumectl-X.Y.Z-linux-x86_64` |
| `aarch64-unknown-linux-gnu` | ubuntu (cross-rs) | `brume-X.Y.Z-aarch64-unknown-linux-gnu` |

The aarch64 Linux artifact is the binary that `brumectl install` pulls
onto a CM5. The three brumectl binaries are what users download from
the GitHub Release page on their workstation.

The workflow extracts the changelog block for `X.Y.Z` and uses it as
the GitHub Release body, so the release notes match what's in
`CHANGELOG.md` byte-for-byte.

### 5c. Phase 3: verify

After CI completes:

- The tag `vX.Y.Z` exists on GitHub.
- `https://github.com/aftertonesignal/brume/releases/tag/vX.Y.Z` lists four
  artifacts and shows the changelog body.
- A fresh `brumectl install` against a CM5 pulls
  `brume-X.Y.Z-aarch64-unknown-linux-gnu` and lands a working Brume.
- `brumectl --version` on a freshly downloaded binary reports the new
  version.

If any of those fails, do not delete the tag. Open an issue, fix
forward in `X.Y.(Z+1)`. Tag deletion poisons any downstream consumer
that has already cached the tag SHA.

## 6. Artifact inventory

Per release, four binaries (see §5b for the table). Distribution is
through GitHub Releases only at v0.3.0; package-manager distribution
(Homebrew tap, apt repo, Flatpak) is deferred until the manual flow
has run a few times and the rough edges are understood.

## 7. Tag and commit signing

**Tag signing.** Start unsigned (`git tag vX.Y.Z`); switch to signed
(`git tag -s vX.Y.Z`) when there's a downstream consumer that
verifies, or a contributor who asks. Maintaining a signing key has a
real cost; matching that cost to a real consumer keeps the procedure
honest.

**Commit signing.** No DCO or GPG signing is enforced today. If we add
anything, it'll be DCO (`Signed-off-by:`), which is low-friction
and matches the Linux kernel + GitLab convention. GPG-signed commits
are a higher bar than is warranted at this scale.

## 8. Pointers

- `DEPLOY.md`: dev-loop and CM5 deploy workflow.
- `INSTALL.md`: end-user install path on a fresh CM5.
- `CHANGELOG.md`: the changelog itself; the `[Unreleased]` block at
  the top is where in-progress work accumulates between cuts.
- `RELEASING.md` (*pending*): the public-facing version of phase 1
  for outside contributors who want to cut a release. Will reference
  this doc for the internal mechanics.
- `CONTRIBUTING.md` (*pending*): public-facing contributor guide;
  links here for "what version is this and when will my change ship?"
