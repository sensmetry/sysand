# Changelog

### v0.1.0 - 2026-06-01

This is a big release. The main changes are the support for the new Sysand Index
at https://sysand.com, and changing the local environment structure to have human
readable directory names. Projects from the new index can now be referred to
by the new `publisher/name` syntax in `sysand add` and `sysand remove`, instead of
an IRI.

Lock file (`sysand-lock.toml`) format has changed, and has to be regenerated
by running `sysand lock`.

Local Sysand environment will now have a different structure and live in
`.sysand`; existing `sysand_env` directories won't be used anymore and can be
removed. To populate the new environment with projects recorded in lock file,
run `sysand sync`.

Index structure has also changed, so all indexes will need to be recreated
using the new `sysand index` commands.

#### API and Breaking Changes

- feat!: implement the new sysand index read protocol (#288)
- feat!: use new local environment structure (#297)
- feat!: rename `sysand_env` to `.sysand` (#336)
- feat!: make all KPAR sources have kpar_digest and src sources have checksum (#352)
- feat!: default to new index and prepare docs for the 0.1.0 release (#356)
- feat!: make standard library identifiers not use `urn:kpar:` (#345)

#### New features added

- feat: support sysand add/remove PURL shorthand (#354)
- feat: add index management commands (#335)
- feat: support cloning local kpars (#363)
- feat: add installation scripts (#370)
- feat: add Dockerfile and automation to publish on/for releases (#362)
- feat: add Java bindings for writing .project.json and .meta.json (#349)
- feat(build): bundle LICENSES into kpar (#332)
- feat(build): bundle CHANGELOG.md into kpar (#331)

#### Enhancements made

- Don't update metadata by default when building projects (#298)
- feat: make built KPARs reproducible by setting OS/time metadata to fixed values (#372)
- Improve 'sysand info' command output (#292)

#### Bugs fixed

- fix: remove old symbols if the same file is included multiple times (#359)

#### Maintenance and upkeep improvements

- feat(dockerimage): provide flexible tags (#365)
- release: prepare 0.1.0-rc.1 (#357)
- release: prepare 0.1.0-alpha.1 (#340)
- refactor: centralize sha256 calculations, update sha2 to 0.11 and gix to 0.83 (#339)
- refactor: don't use test\_ prefix for tests (#322)
- prepare 0.0.12-dev.1 release (#319)
- chore: transition from xz to gzip compression on tars (#369)

#### Documentation improvements

- docs: refine release instructions (#341)
- docs: reduce complexity of RELEASE.md with latest github-activity (#328)
- docs: declare changelog procedure, add backfilled changelog, and introduce commit/PR title practice (#323)
- docs: make navigation sections not expanded (#321)
- Add more details to RELEASE.md (#318)

#### Continuous integration improvements

- ci: release procedure rework (#314)

#### Other merged PRs

- fix(deps): update dependency org.apache.maven:maven-plugin-api to v3.9.16 (#351)
- chore(deps): update all github actions (#350)
- chore(deps): update taiki-e/install-action action to v2.77.5 (#337)
- chore(deps): update dependency org.apache.maven.plugins:maven-invoker-plugin to v3.10.1 (#330)
- chore(deps): update all github actions (#329)
- chore(deps): update dependency org.apache.maven.plugins:maven-invoker-plugin to v3.10.0 (#316)
- chore(deps): update taiki-e/install-action action to v2.75.23 (#315)

### v0.0.11 - 2026-04-30

#### New features

- [workspaces] Add `metadata` field to `.workspace.json` to allow setting metadata for the whole workspace ([#238](https://github.com/sensmetry/sysand/pull/238)). Fields from there are read and replace keys from `.meta.json` when building the kpars. Currently only `metamodel` is supported. As with all workspace functionality, this is expected to change in the future.
- [workspaces] Allow specifying custom `meta.index` for each project in workspace ([#241](https://github.com/sensmetry/sysand/pull/241)). Currently this functionality is only exposed through Java bindings.
- Add `sysand publish` command ([#249](https://github.com/sensmetry/sysand/pull/249)). It allows publishing to an index. Note: this is not ready for use, as `beta.sysand.org` index does not support publishing via an API yet.
- Include `README.md` when building the kpar if the file is present ([#239](https://github.com/sensmetry/sysand/pull/239))
- Add environment metadata file `env.toml`, which resides in `sysand_env` ([#175](https://github.com/sensmetry/sysand/pull/175)). Currently `entries.txt` and per-project `versions.txt` are still also used, but will be removed in the future.
- Change formatting/linting to use pre-commit hooks, and use `prek` to run those hooks in CI instead of a separate implementations in `**/run_chores.sh` and CI ([#250](https://github.com/sensmetry/sysand/pull/250)).
- Move Rust unit tests into separate files; for each file `code.rs`, tests are in `code_tests.rs` ([#282](https://github.com/sensmetry/sysand/pull/282)).

### v0.0.10 - 2026-03-19

#### Added

- Support git usages, using either regular or `git+` IRIs.
- Support for configuring usage sources. See [docs](https://docs.sysand.org/config/dependencies.html) for details.
- Allow adding usages by path: `sysand add --path ../path/to/the/usage`. Both relative and absolute paths
  are supported. Note that this will likely not work for project sharing, since usages are stored in `.project.json` as
  absolute paths, and so are unlikely to be available on other computers at the same location.
- Support publisher field in `.project.json`. This is optional to provide when initializing the project and defaults to
  "untitled". Support for projects without a publisher field remains unchanged.

#### Changed

- Bump minimum supported Python version to 3.10 (from 3.8).

#### Improved

- Support TLS 1.3 for all platforms. Previously only TLS <=1.2 was supported on Windows and macOS.
- Send `User-Agent` header for all HTTP(S) requests. Value is `sysand/<version>`.
- Improved logging, especially for HTTP requests.
- Improved error reporting, especially from the usage/dependency resolvers.

#### Documentation

- Add documentation about [`sysand clone` command](https://docs.sysand.org/commands/clone.html).
- Improve documentation about [`sysand info` command](https://docs.sysand.org/commands/info.html).

### v0.0.9 - 2026-02-03

No changelog entry was written.

### v0.0.8 - 2026-01-22

#### Changes

- When sysand CLI crashes, it now provides a nice message to users that suggests creating a bug report ([#153](https://github.com/sensmetry/sysand/pull/153))

### v0.0.7 - 2025-12-12

#### Changes

- Added `clone` command that enables cloning a package for editing and playing around ([#137](https://github.com/sensmetry/sysand/pull/137)). See `sysand clone --help` for details.
- Unified `init` and `new` commands into just `init`. To get previous behaviour of `sysand new example_project`, use `sysand init example_project`.
- Improved the command for specifying the metamodel ([#128](https://github.com/sensmetry/sysand/pull/128)): now instead of writing the full metamodel URL, one needs to specify only the date.
- Package indexes can now be specified in `sysand.toml` ([#121](https://github.com/sensmetry/sysand/pull/121))
- Unify the style of messages reported by CLI.

### v0.0.6 - 2025-11-14

#### Changes

- Added workspace support ([#101](https://github.com/sensmetry/sysand/pull/101)). _Warning: it is considered experimental and can be changed or removed at any time._
  Current features:
  - Projects belonging to the workspace can be listed in `.workspace.json`.
  - Sysand can build kpar archives for all projects in workspace by running `sysand build` in workspace root directory

### v0.0.5 - 2025-11-07

#### Changes

- Change lockfile to have better structure and rename it from `SysandLock.toml` to `sysand-lock.toml` ([#58](https://github.com/sensmetry/sysand/pull/58)).
  Only the new lockfile format is supported. Lockfile can be regenerated by running `sysand lock`.
- Warn users that SysML/KerML standard library packages are not installed by default to prevent confusion ([#94](https://github.com/sensmetry/sysand/pull/94)).
- Improve error reporting. Errors now report their immediate context, e.g. when failing to read a file, file name is now reported ([#87](https://github.com/sensmetry/sysand/pull/87))
- Add metadata for newer SysML/KerML standard libraries. This speeds up any commands that determine project usages, such as `add`, `sync`, `install`, etc. (part of [#74](https://github.com/sensmetry/sysand/pull/74))

#### Fixes

- Use relative paths in lockfile. This fixes portability to other systems ([#85](https://github.com/sensmetry/sysand/pull/85)).

### v0.0.4 - 2025-10-21

#### Changes

- Fix errors when system temporary directory and `sysand_env` reside on different mount points or drives. ([#81](https://github.com/sensmetry/sysand/pull/81))

### v0.0.3 - 2025-10-17

No changelog entry was written.

### v0.0.2 - 2025-09-18

No changelog entry was written.

### v0.0.1 - 2025-09-17

No changelog entry was written.
