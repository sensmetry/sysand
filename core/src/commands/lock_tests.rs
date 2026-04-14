use std::collections::HashMap;

use crate::{
    commands::lock::{LockError, do_lock_extend},
    lock::{Lock, Project},
    resolve::null::NullResolver,
};

#[test]
fn lock_export_conflict() {
    let exports = vec!["sym1".into(), "sym2".into(), "sym3".into()];

    let lock = Lock {
        lock_version: String::new(),
        projects: vec![
            Project {
                name: Some("test1".into()),
                publisher: None,
                version: String::new(),
                exports: exports.clone(),
                identifiers: vec!["test1".into()],
                checksum: String::new(),
                sources: vec![],
                usages: vec![],
            },
            Project {
                name: Some("test2".into()),
                publisher: None,
                version: String::new(),
                exports,
                identifiers: vec!["test2".into()],
                checksum: String::new(),
                sources: vec![],
                usages: vec![],
            },
        ],
    };
    let res = do_lock_extend(
        lock,
        [],
        NullResolver {},
        &HashMap::new(),
        &Default::default(),
    );

    assert!(matches!(res, Err(LockError::NameCollision(_))));
}
