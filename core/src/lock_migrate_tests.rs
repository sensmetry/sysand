// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::lock::{CURRENT_LOCK_VERSION, LOCKFILE_PREFIX, Lock, Usage};

fn lockfile_0_5() -> String {
    format!(
        r#"{LOCKFILE_PREFIX}lock_version = "0.5"

[[project]]
publisher = "ACME Inc."
name = "App"
version = "1.0.0"
identifiers = ["urn:sysand:ACME%20Inc./App"]
usages = [
    "urn:sysand:ACME%20Inc./Lib",
    "urn:sysand:Ąžuolas/Šaknis",
    "pkg:sysand/acme-labs/util",
    "urn:kpar:other",
]
sources = [{{ editable = "." }}]

[[project]]
publisher = "ACME Inc."
name = "Lib"
version = "1.0.0"
identifiers = [
    "urn:sysand:ACME%20Inc./Lib",
    "urn:kpar:lib",
]
sources = [{{ editable = "lib" }}]

[[project]]
publisher = "Ąžuolas"
name = "Šaknis"
version = "1.0.0"
identifiers = ["urn:sysand:Ąžuolas/Šaknis"]
sources = [{{ editable = "saknis" }}]

[[project]]
publisher = "Acme Labs"
name = "Util"
version = "1.0.0"
identifiers = ["pkg:sysand/acme-labs/util"]
sources = [{{ editable = "util" }}]

[[project]]
name = "Other"
version = "1.0.0"
identifiers = ["urn:kpar:other"]
sources = [{{ editable = "other" }}]
"#
    )
}

#[test]
fn lockfile_0_5_is_migrated_to_current_version() {
    let lock = Lock::parse(&lockfile_0_5()).unwrap();
    assert_eq!(lock.lock_version, CURRENT_LOCK_VERSION);

    let identifiers: Vec<_> = lock
        .projects
        .iter()
        .map(|p| p.identifiers.as_slice())
        .collect();
    assert_eq!(
        identifiers,
        [
            &["urn:sysand:acme-inc/app".to_owned()][..],
            &[
                "urn:sysand:acme-inc/lib".to_owned(),
                "urn:kpar:lib".to_owned()
            ],
            &["urn:sysand:ąžuolas/šaknis".to_owned()],
            &["pkg:sysand/acme-labs/util".to_owned()],
            &["urn:kpar:other".to_owned()],
        ]
    );
    assert_eq!(
        lock.projects[0].usages,
        [
            Usage::from_str_unchecked("urn:sysand:acme-inc/lib"),
            Usage::from_str_unchecked("urn:sysand:ąžuolas/šaknis"),
            Usage::from_str_unchecked("pkg:sysand/acme-labs/util"),
            Usage::from_str_unchecked("urn:kpar:other"),
        ]
    );
}

#[test]
fn migrated_lockfile_is_written_as_current_version() {
    let text = Lock::parse(&lockfile_0_5()).unwrap().to_toml().to_string();
    assert!(
        text.starts_with(&format!(
            "{LOCKFILE_PREFIX}lock_version = \"{CURRENT_LOCK_VERSION}\"\n"
        )),
        "{text}"
    );
    assert!(!text.contains("ACME%20Inc."), "{text}");
    // Migrating the result changes nothing
    assert_eq!(Lock::parse(&text).unwrap().to_toml().to_string(), text);
}

#[test]
fn current_lockfile_is_not_migrated() {
    // Identifiers as 0.5 derived them are taken as they are in a lockfile of
    // the current version
    let lockfile = lockfile_0_5().replace(
        r#"lock_version = "0.5""#,
        &format!(r#"lock_version = "{CURRENT_LOCK_VERSION}""#),
    );
    let lock = Lock::parse(&lockfile).unwrap();
    assert_eq!(
        lock.projects[0].identifiers,
        ["urn:sysand:ACME%20Inc./App".to_owned()]
    );
}
