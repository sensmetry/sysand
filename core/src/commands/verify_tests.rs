// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! `do_verify` against a real `.sysand` in a temp directory.

use std::{collections::HashMap, convert::Infallible, io::Write as _, num::NonZeroU64};

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::{Utf8TempDir, tempdir};
use typed_path::Utf8UnixPathBuf;

use super::*;
use crate::{
    commands::sync::{SyncOutcome, do_sync},
    env::{WriteEnvironment as _, local_directory::LocalDirectoryEnvironment},
    lock::{Project, Usage},
    project::{
        KparMeta,
        local_kpar::{KparInnerPath, LocalKParProject},
        memory::InMemoryProject,
    },
    utils::sha256_lowercase_hex,
};

type NoRemoteSrc = fn(String, String) -> Result<InMemoryProject, Infallible>;
type NoRemoteKpar = fn(String, NonZeroU64, String) -> Result<InMemoryProject, Infallible>;
type NoGit = fn(String) -> Result<InMemoryProject, Infallible>;

/// A dependency project directory: one model file, checksummed in
/// `.meta.json`.
fn write_dep(dir: &Utf8Path, name: &str, version: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let model = format!("package {name} {{ }}\n");
    let file = format!("{name}.sysml");
    std::fs::write(dir.join(&file), &model).unwrap();
    std::fs::write(
        dir.join(".project.json"),
        format!(r#"{{"name":"{name}","publisher":"acme","version":"{version}"}}"#),
    )
    .unwrap();
    std::fs::write(
        dir.join(".meta.json"),
        format!(
            r#"{{"index":{{"{name}":"{file}"}},"created":"2026-01-01T00:00:00Z","checksum":{{"{file}":{{"algorithm":"SHA256","value":"{}"}}}}}}"#,
            sha256_lowercase_hex(&model)
        ),
    )
    .unwrap();
}

/// The root project: `.project.json` using `urn:dep<i>` with `^1.0.0`.
fn write_app(dir: &Utf8Path, deps: &[String], constraint: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let usages: Vec<String> = deps
        .iter()
        .map(|dep| format!(r#"{{"resource":"urn:{dep}","versionConstraint":"{constraint}"}}"#))
        .collect();
    std::fs::write(
        dir.join(".project.json"),
        format!(
            r#"{{"name":"app","publisher":"acme","version":"0.1.0","usage":[{}]}}"#,
            usages.join(",")
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join(".meta.json"),
        r#"{"index":{},"created":"2026-01-01T00:00:00Z"}"#,
    )
    .unwrap();
}

fn dep_project(name: &str, sources: Vec<Source>) -> Project {
    Project {
        publisher: Some("acme".to_owned()),
        name: name.to_owned(),
        version: "1.0.0".to_owned(),
        metamodel: None,
        exports: vec![],
        identifiers: vec![format!("urn:{name}")],
        usages: vec![],
        sources,
    }
}

fn app_project(deps: &[String]) -> Project {
    Project {
        publisher: Some("acme".to_owned()),
        name: "app".to_owned(),
        version: "0.1.0".to_owned(),
        metamodel: None,
        exports: vec![],
        identifiers: vec!["urn:app".to_owned()],
        usages: deps
            .iter()
            .map(|dep| Usage::from_str_unchecked(&format!("urn:{dep}")))
            .collect(),
        sources: vec![Source::Editable {
            editable: Utf8UnixPathBuf::from("app"),
        }],
    }
}

struct Fixture {
    _dir: Utf8TempDir,
    root: Utf8PathBuf,
    lock: Lock,
    deps: Vec<String>,
}

impl Fixture {
    /// `n` source dependencies under `deps/`, an app using all of them,
    /// a lockfile, and a synced `.sysand`.
    fn new(n: usize) -> Self {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let deps: Vec<String> = (0..n).map(|i| format!("dep{i}")).collect();
        let mut projects = vec![app_project(&deps)];
        for dep in &deps {
            let dep_dir = root.join("deps").join(dep);
            write_dep(&dep_dir, dep, "1.0.0");
            let checksum = LocalSrcProject::new_access(&dep_dir, None)
                .checksum_canonical_hex()
                .unwrap()
                .unwrap();
            projects.push(dep_project(
                dep,
                vec![Source::LocalSrc {
                    src_path: Utf8UnixPathBuf::from(format!("deps/{dep}")),
                    checksum,
                }],
            ));
        }
        write_app(&root.join("app"), &deps, "^1.0.0");
        let lock = Lock {
            projects,
            ..Lock::default()
        };
        let fixture = Self {
            _dir: dir,
            root,
            lock,
            deps,
        };
        let mut env = fixture.create_env();
        fixture.sync(&mut env);
        fixture
    }

    fn create_env(&self) -> LocalDirectoryEnvironment {
        std::fs::create_dir_all(self.root.join(".sysand")).unwrap();
        LocalDirectoryEnvironment::create(self.root.join(".sysand")).unwrap()
    }

    fn env(&self) -> LocalDirectoryEnvironment {
        LocalDirectoryEnvironment::read(self.root.join(".sysand")).unwrap()
    }

    fn sync(&self, env: &mut LocalDirectoryEnvironment) -> SyncOutcome {
        let root = self.root.clone();
        let kpar_root = self.root.clone();
        let mut outcome = SyncOutcome::default();
        do_sync(
            &self.lock,
            env,
            Some(|src_path: Utf8UnixPathBuf, checksum: String| {
                LocalSrcProject::new_access_with_checksum(
                    root.join(src_path.as_str()),
                    Some(src_path),
                    checksum,
                )
            }),
            None::<NoRemoteSrc>,
            Some(
                |kpar_path: Utf8UnixPathBuf, kpar_size: NonZeroU64, kpar_digest: String| {
                    LocalKParProject::new_with_expected(
                        kpar_root.join(kpar_path.as_str()),
                        KparInnerPath::Guess,
                        Some(kpar_path),
                        KparMeta {
                            size_bytes: kpar_size,
                            sha256_hex: kpar_digest,
                        },
                    )
                },
            ),
            None::<NoRemoteKpar>,
            None::<NoRemoteKpar>,
            None::<NoGit>,
            &HashMap::default(),
            false,
            &mut outcome,
        )
        .unwrap();
        outcome
    }

    fn manifests(&self) -> Vec<ManifestUsages> {
        vec![
            ManifestUsages::read(&LocalSrcProject::new_access(self.root.join("app"), None))
                .unwrap(),
        ]
    }

    fn verify(&self) -> VerifyReport {
        let env = self.env();
        do_verify(
            &self.lock,
            Some(&env),
            &self.manifests(),
            &HashMap::default(),
        )
        .unwrap()
    }

    /// The installed copy of `dep`.
    fn installed(&self, dep: &str) -> Utf8PathBuf {
        let env = self.env();
        let entry = env
            .projects()
            .iter()
            .find(|p| p.name == dep)
            .expect("installed");
        env.root_path().join(entry.path.as_str())
    }
}

fn kinds(report: &VerifyReport) -> Vec<(String, MismatchKind)> {
    report
        .mismatched
        .iter()
        .map(|m| (m.iri.clone(), m.kind.clone()))
        .collect()
}

#[test]
fn clean_environment_verifies() {
    let fixture = Fixture::new(3);
    let report = fixture.verify();
    assert!(report.is_clean(), "{:?}", report.findings());
    assert_eq!(report.verified.len(), 3);
    assert_eq!(report.problems(), 0);
}

#[test]
fn tampered_model_file_mismatches() {
    let fixture = Fixture::new(2);
    let installed = fixture.installed("dep1");
    std::fs::write(installed.join("dep1.sysml"), "package dep1 { part p; }\n").unwrap();
    let report = fixture.verify();
    assert_eq!(
        kinds(&report),
        vec![(
            "urn:dep1".to_owned(),
            MismatchKind::Checksum {
                file: Some(Utf8UnixPathBuf::from("dep1.sysml"))
            }
        )]
    );
    assert_eq!(report.verified.len(), 1);
    let text = report.mismatched[0].to_string();
    assert!(
        text.starts_with("installed project `urn:dep1` does not match the lockfile checksum"),
        "{text}"
    );
    assert!(text.contains("`dep1.sysml` differs"), "{text}");
}

#[test]
fn tampered_manifest_mismatches() {
    let fixture = Fixture::new(1);
    let installed = fixture.installed("dep0");
    std::fs::write(
        installed.join(".project.json"),
        r#"{"name":"dep0","publisher":"acme","version":"1.0.0","description":"edited"}"#,
    )
    .unwrap();
    let report = fixture.verify();
    assert_eq!(
        kinds(&report),
        vec![("urn:dep0".to_owned(), MismatchKind::Checksum { file: None })]
    );
    assert_eq!(
        report.mismatched[0].to_string(),
        "installed project `urn:dep0` does not match the lockfile checksum"
    );
}

#[test]
fn kpar_install_tampered_file_mismatches() {
    // A dependency installed from an archive: the lockfile knows only
    // the archive digest, so the per-file digests of its `.meta.json`
    // are what verification hashes.
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    write_dep(&root.join("src-lib"), "lib", "1.0.0");
    let archive = root.join("lib.kpar");
    {
        let file = std::fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for name in [".project.json", ".meta.json", "lib.sysml"] {
            zip.start_file(name, options).unwrap();
            zip.write_all(&std::fs::read(root.join("src-lib").join(name)).unwrap())
                .unwrap();
        }
        zip.finish().unwrap();
    }
    let bytes = std::fs::read(&archive).unwrap();
    let lock = Lock {
        projects: vec![
            app_project(&["lib".to_owned()]),
            dep_project(
                "lib",
                vec![Source::LocalKpar {
                    kpar_path: Utf8UnixPathBuf::from("lib.kpar"),
                    kpar_size: NonZeroU64::new(bytes.len() as u64).unwrap(),
                    kpar_digest: sha256_lowercase_hex(&bytes),
                }],
            ),
        ],
        ..Lock::default()
    };
    write_app(&root.join("app"), &["lib".to_owned()], "^1.0.0");
    let fixture = Fixture {
        _dir: dir,
        root,
        lock,
        deps: vec!["lib".to_owned()],
    };
    let mut env = fixture.create_env();
    fixture.sync(&mut env);
    assert!(fixture.verify().is_clean());

    let installed = fixture.installed("lib");
    std::fs::write(installed.join("lib.sysml"), "tampered").unwrap();
    let report = fixture.verify();
    assert_eq!(
        kinds(&report),
        vec![(
            "urn:lib".to_owned(),
            MismatchKind::Checksum {
                file: Some(Utf8UnixPathBuf::from("lib.sysml"))
            }
        )]
    );
    let _ = &fixture.deps;
}

#[test]
fn missing_directory_is_missing_from_disk() {
    let fixture = Fixture::new(2);
    std::fs::remove_dir_all(fixture.installed("dep0")).unwrap();
    let report = fixture.verify();
    assert_eq!(
        kinds(&report),
        vec![("urn:dep0".to_owned(), MismatchKind::MissingFromDisk)]
    );
    assert_eq!(
        report.mismatched[0].to_string(),
        "`env.toml` records `urn:dep0`, which is not installed"
    );
}

#[test]
fn truncated_project_is_missing_from_disk() {
    let fixture = Fixture::new(1);
    std::fs::remove_file(fixture.installed("dep0").join(".meta.json")).unwrap();
    let report = fixture.verify();
    assert_eq!(
        kinds(&report),
        vec![("urn:dep0".to_owned(), MismatchKind::MissingFromDisk)]
    );
}

#[test]
fn unrecorded_directory_is_listed() {
    let fixture = Fixture::new(1);
    let stray = fixture.root.join(".sysand/lib/kpar.stray_1.0.0");
    std::fs::create_dir_all(&stray).unwrap();
    std::fs::write(stray.join("x.sysml"), "").unwrap();
    let report = fixture.verify();
    assert_eq!(
        report.mismatched,
        [] as [crate::commands::sync::Mismatch; 0]
    );
    assert_eq!(
        report.unrecorded,
        vec![Utf8UnixPathBuf::from("lib/kpar.stray_1.0.0")]
    );
    assert_eq!(
        report.findings(),
        vec!["`lib/kpar.stray_1.0.0` is installed but not recorded in `env.toml`"]
    );
}

#[test]
fn locked_but_not_installed_is_not_recorded() {
    let fixture = Fixture::new(2);
    let mut env = fixture.env();
    env.del_project_version("urn:dep1", "1.0.0").unwrap();
    let report = fixture.verify();
    assert_eq!(
        kinds(&report),
        vec![("urn:dep1".to_owned(), MismatchKind::NotRecorded)]
    );
    assert_eq!(
        report.mismatched[0].to_string(),
        "`urn:dep1` 1.0.0 is in `sysand-lock.toml` but not installed"
    );

    // No environment at all: everything is not installed.
    let report = do_verify(
        &fixture.lock,
        None,
        &fixture.manifests(),
        &HashMap::default(),
    )
    .unwrap();
    assert_eq!(report.mismatched.len(), 2);
    assert!(
        report
            .mismatched
            .iter()
            .all(|m| m.kind == MismatchKind::NotRecorded)
    );
}

#[test]
fn stale_constraint_rejects_locked_version() {
    let fixture = Fixture::new(1);
    write_app(&fixture.root.join("app"), &fixture.deps, "^2.0.0");
    let report = fixture.verify();
    assert_eq!(
        report.mismatched,
        [] as [crate::commands::sync::Mismatch; 0]
    );
    assert_eq!(report.stale.len(), 1);
    assert_eq!(
        report.stale[0].to_string(),
        format!(
            "the lockfile does not match the manifests: `urn:dep0` is `^2.0.0` in `{}` and `1.0.0` in `sysand-lock.toml`",
            fixture.root.join("app/.project.json")
        )
    );
}

#[test]
fn stale_usage_added_and_removed() {
    let fixture = Fixture::new(2);
    // The manifest uses `dep0` and a new `dep9`, and not `dep1`.
    write_app(
        &fixture.root.join("app"),
        &["dep0".to_owned(), "dep9".to_owned()],
        "^1.0.0",
    );
    let report = fixture.verify();
    let kinds: Vec<(String, StaleKind)> = report
        .stale
        .iter()
        .map(|s| (s.usage.clone(), s.kind.clone()))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("urn:dep9".to_owned(), StaleKind::Added),
            ("urn:dep1".to_owned(), StaleKind::Removed),
        ]
    );
    let manifest = fixture.root.join("app/.project.json");
    assert_eq!(
        report.stale[0].to_string(),
        format!(
            "the lockfile does not match the manifests: `urn:dep9` is in `{manifest}` and not in `sysand-lock.toml`"
        )
    );
    assert_eq!(
        report.stale[1].to_string(),
        format!(
            "the lockfile does not match the manifests: `urn:dep1` is in `sysand-lock.toml` and no longer in `{manifest}`"
        )
    );
}

#[test]
fn manifest_without_lock_entry_is_stale() {
    let mut fixture = Fixture::new(1);
    fixture.lock.projects.retain(|p| p.name != "app");
    let report = fixture.verify();
    assert_eq!(report.stale.len(), 1);
    assert_eq!(report.stale[0].kind, StaleKind::ManifestMissing);
    assert!(
        report.stale[0]
            .to_string()
            .ends_with("is not in `sysand-lock.toml`")
    );
}

#[test]
fn verify_cost_100_projects() {
    let fixture = Fixture::new(100);
    let started = std::time::Instant::now();
    let report = fixture.verify();
    let elapsed = started.elapsed();
    assert!(report.is_clean());
    assert_eq!(report.verified.len(), 100);
    eprintln!(
        "verify of 100 installed projects: {} ms",
        elapsed.as_millis()
    );
}

/// The usages of a `.project.json` in a fresh directory.
fn read_usages(usages: &str) -> Result<ManifestUsages, VerifyError> {
    let dir = tempdir().unwrap();
    std::fs::write(
        dir.path().join(".project.json"),
        format!(r#"{{"name":"app","publisher":"acme","version":"0.1.0","usage":[{usages}]}}"#),
    )
    .unwrap();
    std::fs::write(
        dir.path().join(".meta.json"),
        r#"{"index":{},"created":"2026-01-01T00:00:00Z"}"#,
    )
    .unwrap();
    ManifestUsages::read(&LocalSrcProject::new_access(dir.path(), None))
}

#[test]
fn an_index_usage_keeps_its_constraint() {
    let manifest =
        read_usages(r#"{"publisher":"acme","name":"widgets","versionConstraint":"^2.0.0"}"#)
            .unwrap();
    assert_eq!(manifest.usages.len(), 1);
    assert_eq!(
        manifest.usages[0].identifier.as_str(),
        "pkg:sysand/acme/widgets"
    );
    assert_eq!(
        manifest.usages[0].constraint,
        Some(VersionReq::parse("^2.0.0").unwrap())
    );
}

#[test]
fn an_unparsable_constraint_is_an_error() {
    let error =
        read_usages(r#"{"resource":"urn:dep0","versionConstraint":"not a version"}"#).unwrap_err();
    assert!(
        matches!(
            error,
            VerifyError::InvalidUsage {
                manifest: _,
                source: _
            }
        ),
        "{error:?}"
    );
}
