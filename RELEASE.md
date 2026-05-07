# Releasing Sysand

The release procedure is coordinated with the [sysand-signing] GitLab project.
Its automation can promote a `nightly` release with unsigned Rust binaries to a
versioned release with signed and packaged Rust binaries.

To make a versioned release, follow this procedure:

1. Create a PR titled "Prepare release for vX.Y.Z".
   1. Add a `docs/src/changelog.md` entry ([changelog-instructions]).
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
- Documentation is deployed by this repository's mdBook workflow when the final
  non-prerelease GitHub Release is published.

[sysand-signing]: https://gitlab.com/sensmetry/internal2/tech/syside/sysand/sysand-signing
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

2. Update your local git repository.

   ```sh
   # verify "origin" references sensmetry/sysand and not a fork
   git remote -v

   # update your local main from origin's main
   git checkout main
   git pull origin main
   git log

   # update local tags to match origin's tags
   git fetch origin --tags --prune --prune-tags --force
   ```

3. Iteratively generate a changelog entry.
   1. Generate an initial changelog entry.

      ```sh
      github-activity --heading-level=3
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
      - `docs` - for changes that only touch docs/ or other markdown files

      If any PR introduces a breaking change that does not have the `breaking`
      label, add it as well.

   3. Revise PR titles.

      If you find PR titles that breaks our convention in [DEVELOPMENT.md], you
      could re-title them at this point.

   4. Generate a final changelog.

      ```sh
      github-activity --heading-level=3
      ```

4. Add it to `docs/src/changelog.md` and make final manual edits.
   - Add it under a section like `## vX.Y`
   - Update title to `### vX.Y.Z - YYYY-MM-DD`
   - Remove the "(full changelog)" link
   - Write a leading paragraph summarizing the release

[`github-activity`]: https://pypi.org/project/github-activity/
[development.md]: DEVELOPMENT.md

### Determine the next version

We aim to follow the [SemVer 2] versioning scheme.

With a preliminary changelog, we can determine what the next version should be
based on the following rules:

- increment the major version if a breaking change has been made
- increment the minor version if enhancements or new features have been added
- increment the patch version otherwise

[semver 2]: https://semver.org/

### Bump version entries

1. Update the `version` entry in `bindings/js/package.json`
2. Update the `workspace.package.version` entry in `Cargo.toml`
3. Run `cargo generate-lockfile`
