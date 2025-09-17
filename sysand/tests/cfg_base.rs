// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use assert_cmd::prelude::*;
use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

#[test]
fn cfg_set_quiet() -> Result<(), Box<dyn std::error::Error>> {
    let (_, _, out_normal) = run_sysand(&vec!["new", "cfg_set_quiet"], None)?;

    out_normal
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "Creating interchange project `cfg_set_quiet`",
        ));

    let (_, _, out_quiet_flag) = run_sysand(&vec!["new", "--quiet", "cfg_set_quiet"], None)?;

    out_quiet_flag
        .assert()
        .success()
        .stderr(predicate::str::contains("Creating interchange project `cfg_set_quiet`").not());

    let (_temp_dir, cwd) = new_temp_cwd()?;

    let quiet_cfg = toml::to_string(&sysand_core::config::Config {
        quiet: Some(true),
        verbose: None,
        index: None,
    })?;

    let cfg_path = cwd.join(sysand_core::config::local_fs::CONFIG_FILE);

    std::fs::write(&cfg_path, quiet_cfg.clone())?;

    let out_quiet_local_config =
        run_sysand_in(&cwd, &vec!["new", "cfg_set_quiet"], cfg_path.to_str())?;

    out_quiet_local_config
        .assert()
        .success()
        .stderr(predicate::str::contains("Creating interchange project `cfg_set_quiet`").not());

    Ok(())
}
