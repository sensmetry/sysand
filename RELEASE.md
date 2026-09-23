# Releasing Sysand

The release procedure is coordinated with the [sysand-signing] GitLab project.
Its automation can promote a `nightly` release with unsigned Rust binaries to a
versioned release with signed and packaged Rust binaries.

To make a versioned release, follow this procedure:

1. Create a PR titled "Prepare release for vX.Y.Z".
   1. Prepare a changelog draft ([changelog-instructions]).
   2. Determine the next version ([versioning-instructions]).
   3. Bump version entries ([bump-instructions]).
2. When the PR is reviewed and merged, wait for the `main` CI pipelines to
   recreate the `nightly` GitHub Release with its unsigned `sysand` binaries.
3. Follow the release instructions in the [sysand-signing] GitLab project's
   README.md file, then continue here.
4. Verify a `vX.Y.Z` git tag and an associated [GitHub Release] exist in this
   repository, and that [GitHub workflows] was triggered and has not failed.
5. Approve the `Release to PyPI` GitHub workflow job, which should now be
   pending approval, and verify it passes.
6. Approve the release to Maven by pressing "Go live" in [Maven Central], where
   you need access to the Sensmetry organization.
7. Announce the release in the [forum], reusing content from the changelog and
   aligning with the format of past release announcements.

The final version release has several publication paths:

- CLI binaries are built by this repository's `Rust (Library & CLI)` workflow
  on `main`, attached to the `nightly` GitHub Release as raw signing inputs,
  then signed, packaged, and published to the public GitHub Release by
  `sysand-signing`.
- Python packages are built and published to PyPI by this repository's
  `Python (Bindings)` workflow when `sysand-signing` creates the final `v*`
  release tag.
- Java artifacts are built and deployed to Maven Central by this repository's
  `Java (Bindings)` workflow when `sysand-signing` creates the final `v*`
  release tag.
- Documentation is maintained and deployed by the [sysand-index] GitLab project.
  This repository still owns the technical design notes in `design/`.

[sysand-signing]: https://gitlab.com/sensmetry/internal2/tech/syside/sysand/sysand-signing
[sysand-index]: https://gitlab.com/sensmetry/internal2/tech/syside/sysand/index-website
[changelog-instructions]: #add-a-changelog-entry
[versioning-instructions]: #determine-the-next-version
[bump-instructions]: #bump-version-entries
[github release]: https://github.com/sensmetry/sysand/releases
[github workflows]: https://github.com/sensmetry/sysand/actions
[maven central]: https://central.sonatype.com/publishing
[forum]: https://forum.sensmetry.com/c/sysand/changelogs/30

## Specific instructions

### Add a changelog entry

We generate changelogs from merged PRs using their titles and labels with the
[`github-activity`] tool.

1. Install [`github-activity`].

   ```sh
   # example using pip
   pip install github-activity
   ```

2. Iteratively generate a changelog entry.
   1. Generate an initial changelog entry.

      ```sh
      github-activity --heading-level=2
      ```

   2. Revise PR labels.

      Any PR listed under `Other merged PRs` does not have a categorizable label.
      If it is not just a dependency bump, visit the PR and add _one_ suitable
      label to it among the labels below.
      - `bug` - for user-impacting bug fixes
      - `new` - for user-impacting new functionality
      - `enhancement` - for user-impacting enhanced functionality
      - `maintenance` - for all other code-touching changes
      - `ci` - for changes that only touch the CI system
      - `docs` - for changes that only touch `design/` or other markdown files

      If any PR introduces a breaking change that does not have the `breaking`
      label, add it as well.

   3. Revise PR titles.

      If you find PR titles that breaks our convention in [DEVELOPMENT.md], you
      could re-title them at this point.

   4. Generate a final changelog.

      ```sh
      github-activity --heading-level=2
      ```

3. Add it to the sysand client changelog maintained in the [sysand-index]
   repository and make final manual edits.
   - Add it under a section like `## vX.Y`.
   - Update title to `### vX.Y.Z - YYYY-MM-DD`.
   - Remove the "(full changelog)" link.
   - Write a leading paragraph summarizing the release.

[`github-activity`]: https://pypi.org/project/github-activity/
[development.md]: DEVELOPMENT.md

### Determine the next version

We aim to follow the [SemVer 2] versioning scheme.

With a preliminary changelog, we can determine what the next version should be
based on the following rules:

- increment the major version if a breaking change has been made
- increment the minor version if enhancements or new features have been added
- increment the patch version otherwise

#### What the version number covers

A change is breaking if it breaks one of these two surfaces:

- **The CLI.** Command names, their arguments and their exit codes. The
  wording of human-readable output is not part of the promise: a message may
  be reworded in any release, as long as the command still succeeds or fails
  under the same conditions.
- **The Python API.** The public names exported from the `sysand` package on
  PyPI, and the keyword parameter names of its functions.

Everything else is outside it:

- **`sysand experimental`.** Every subcommand under it, and everything named
  with an `Exp`/`exp_` prefix, may change incompatibly or disappear in any
  release, including a patch release. This is the same disclaimer that
  `sysand experimental --help` prints, and it is what lets new usage kinds be
  developed in the open before their commands are committed to.
- **The Rust crates.** `sysand-core`, `sysand`, `sysand-macros` and the
  binding crates set `publish = false` in the workspace `Cargo.toml` and are
  not on crates.io. Their library APIs are internal to this repository:
  renaming an item, changing a signature or adding an enum variant in them is
  not a breaking change.
- **The Java and JavaScript bindings.** Published, but placeholders with no
  known users. Their APIs carry no compatibility promise yet. Say so here
  before that stops being true.

#### File formats are not covered by the version number at all

`.project.json`, `.meta.json`, `sysand-lock.toml` and the environment
metadata are read by other tools, and by sysand binaries installed elsewhere
that no version number reaches. Breaking one of them cannot be repaired by a
major bump, so treat every change to them as permanent.

What already protects them, and must keep working:

- `sysand-lock.toml` carries `lock_version` and the environment metadata
  carries `version`. A value this build does not support is refused with a
  message saying how to regenerate the file, rather than misparsed. Any
  change to the shape of either file needs its version bumped in the same
  commit.
- `.project.json` and `.meta.json` follow the KerML interchange format and
  carry no version of their own. Unknown fields in them are ignored, so
  adding a new optional field is always safe. Do not add
  `#[serde(deny_unknown_fields)]` to any type that either file deserializes
  into.
- A `usage` entry matching no kind a build knows deserializes into
  `InterchangeProjectUsageG::Unknown`, which holds its JSON verbatim. Such a
  project can still be read and edited, and rewriting the manifest puts the
  entry back untouched; only an operation that would act on it as a
  dependency refuses. Keep it that way, so that introducing a fourth usage
  kind never costs older sysands the ability to read a manifest. Builds
  released before this one fail to parse one outright, which is what sets
  the floor for when a new kind can be introduced.

[semver 2]: https://semver.org/

#### What the version number covers

Note: "release" in this section means an official release. This excludes
nightly releases and pre-releases.

A change is breaking if it breaks one of these two surfaces:

- **The CLI.** Command names, their arguments and their exit codes. The
  wording of human-readable output is not part of the promise: a message may
  be reworded in any release, as long as the command still succeeds or fails
  under the same conditions.
- **The Python API.** The public names exported from the `sysand` package on
  PyPI, and the keyword parameter names of its functions.

Everything else is outside it:

- **`sysand experimental`.** Every subcommand under it, and everything named
  with an `Exp`/`exp_` prefix, may change incompatibly or disappear in any
  release, including a patch release. This is the same disclaimer that
  `sysand experimental --help` prints, and it is what lets new usage kinds be
  developed in the open before their commands are committed to.
- **Rust crates.** `sysand-core`, `sysand`, `sysand-macros` are not on
  crates.io. Their library APIs are internal to this repository and any changes
  can be made at any time.
- **Java bindings.** Published to Maven, but with a single known user
  ([SysML v2 Pilot](https://github.com/Systems-Modeling/SysML-v2-Pilot-Implementation))
  whose interaction with sysand we maintain. API carries no compatibility
  promise yet.
- **JavaScript bindings.** Essentially a placeholder, not published and no known
  users. API carries no compatibility promise yet.

#### File formats

`.project.json`, `.meta.json`, `sysand-lock.toml` and `env.toml` are versioned
separately:

- `sysand-lock.toml` carries `lock_version` and `env.toml` has `version`.
  Sysand currently supports only the latest version it knows about, any
  unsupported versions always produce an error on read.
  Any change to the shape (or semantics) of either file needs its version
  bumped before a release is made.
- `.project.json` and `.meta.json` mostly follow the KerML interchange
  format (with some Sysand additions) and carry no version of their own.
  Unknown fields in them are and must continue to be ignored (they may
  produce a warning, however), so adding a new optional field is always
  safe.
- A new `usage` kind in `.project.json` is the one change these rules do not
  make safe: the usage list deserializes through an untagged enum with no
  catch-all, so a manifest declaring a kind a build does not know fails to
  parse in full. Therefore, once a usage kind has been added (and can be
  populated with a non-experimental command), it must never be removed.

### Bump version entries

We look to bump our own declared versions, and do a last minute security audit
of dependencies for known vulnerabilities and fix them if possible.

1. Update `workspace.package.version` in `Cargo.toml`, and the lockfile by
   doing:

   ```sh
   # updates lockfile minimalistically
   cargo check

   # do a last minute security audit
   cargo audit

   # and bump manually to address known vulnerabilities
   cargo update -p vulnerable-crate-1 [--precise 1.2.3]
   ```

2. Update `version` in `bindings/js/package.json` manually, and the lockfile by
   doing:

   ```sh
   cd bindings/js/

   # updates lockfile minimalistically
   npm install --package-lock-only --ignore-scripts

   # do a last minute security audit
   npm audit

   # and bump minimalistically to address known vulnerabilities
   npm audit fix
   ```

3. Audit the Python package dependencies:

   ```sh
   cd bindings/py
   uv audit
   ```
