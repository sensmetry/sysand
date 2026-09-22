// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Every checked-in model file and manifest is `lf`.
//!
//! `.gitattributes` pins `*.sysml`, `*.kerml`, `*.toml` and `*.json` to
//! `lf`, because fixtures carry per-file SHA256s in committed metadata
//! and goldens, and a checkout that converted them would be green on
//! Linux and red on Windows. This is the guard for the pin: it walks
//! the whole checkout, since the model files live under
//! `sysand/tests/data` and `bindings/java/plugin/src/it`, and the
//! manifests everywhere.

use std::path::{Path, PathBuf};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name == "target"
                || name == "node_modules"
                || name == ".git"
                || name.starts_with(".venv")
                || name == "browser_dist"
                || name == "browser_pkg"
                || name == "browser_test_dist"
            {
                continue;
            }
            collect(&path, out);
        } else if matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("sysml" | "kerml" | "toml" | "json")
        ) {
            out.push(path);
        }
    }
}

#[test]
fn every_checked_in_model_file_and_manifest_is_lf() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    collect(&root, &mut files);
    assert!(
        files.len() >= 7,
        "the walk found the fixtures: {}",
        files.len()
    );

    let offenders: Vec<String> = files
        .iter()
        .filter(|path| std::fs::read(path).is_ok_and(|bytes| bytes.contains(&b'\r')))
        .map(|path| path.display().to_string())
        .collect();
    assert!(
        offenders.is_empty(),
        "files with a carriage return (see .gitattributes): {offenders:#?}"
    );
}
