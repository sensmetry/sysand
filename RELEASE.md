# Releasing Sysand

The release procedure is coordinated with the
[sysand-signing GitLab project][sysand-signing]. That project is the source of
truth for signing, packaging, and publishing GitHub Release assets.

At a high level, a Sysand version release starts in this repository:

1. Prepare a pull request that updates the version number and any generated
   lockfiles that must change with it ([example](https://github.com/sensmetry/sysand/pull/310)).
2. Get the release pull request reviewed and merged to `main`.
3. Wait for the `main` CI pipelines to recreate the `nightly` GitHub Release.
   The `nightly` release contains the raw CLI binaries used as signing inputs.
4. Continue with the release instructions in the
   [sysand-signing GitLab project][sysand-signing]. The signing pipeline is
   responsible for creating the final `v*` release tag.

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
