// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

#[cfg(not(target_os = "windows"))]
use rexpect::session::{PtySession, spawn_command};
#[cfg(not(target_os = "windows"))]
use std::os::unix::process::ExitStatusExt;
use std::{
    error::Error,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;

pub fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join(name)
}

pub fn sysand_cmd_in<'a, I: IntoIterator<Item = &'a str>>(
    cwd: &Path,
    args: I,
    cfg: Option<&str>,
) -> Result<Command, Box<dyn Error>> {
    let cfg_args = if let Some(config) = cfg {
        let config_path = cwd.join("sysand.toml");
        let mut config_file = std::fs::File::create_new(&config_path)?;
        config_file.write_all(config.as_bytes())?;
        vec![
            "--config-file".to_string(),
            config_path.display().to_string(),
        ]
    } else {
        vec![]
    };
    let args = [
        args.into_iter().map(|s| s.to_string()).collect(),
        vec!["--no-config".to_string()],
        cfg_args,
    ]
    .concat();
    // NOTE had trouble getting test-temp-dir crate working, but would be better
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("sysand"));

    cmd.env("NO_COLOR", "1");

    cmd.args(args);

    cmd.current_dir(cwd);

    Ok(cmd)
}

/// Creates a temporary directory and returns the tuple of the temporary
/// directory handle and the canonicalised path to it. We need to canonicalise
/// the path because tests check the output of CLI to see whether it operated on
/// the expected files and CLI typically prints the canonicalised version of the
/// path.
pub fn new_temp_cwd() -> Result<(TempDir, PathBuf), Box<dyn Error>> {
    let temp_dir = TempDir::with_prefix("sysand_test_")?;
    let temp_dir_path = temp_dir.path().canonicalize()?;

    Ok((temp_dir, temp_dir_path))
}

pub fn sysand_cmd<'a, I: IntoIterator<Item = &'a str>>(
    args: I,
    cfg: Option<&str>,
) -> Result<(TempDir, PathBuf, Command), Box<dyn Error>> {
    // NOTE had trouble getting test-temp-dir crate working, but would be better
    let (temp_dir, cwd) = new_temp_cwd()?;
    let cmd = sysand_cmd_in(&cwd, args /*, stdin*/, cfg)?;

    Ok((temp_dir, cwd, cmd))
}

pub fn run_sysand_in<'a, I: IntoIterator<Item = &'a str>>(
    cwd: &Path,
    args: I,
    cfg: Option<&str>,
) -> Result<Output, Box<dyn Error>> {
    Ok(sysand_cmd_in(cwd, args, cfg)?.output()?)
}

pub fn run_sysand<'a, I: IntoIterator<Item = &'a str>>(
    args: I,
    cfg: Option<&str>,
) -> Result<(TempDir, PathBuf, Output), Box<dyn Error>> {
    let (temp_dir, cwd, mut cmd) = sysand_cmd(args /*, stdin*/, cfg)?;

    Ok((temp_dir, cwd, cmd.output()?))
}

// TODO: Figure out how to do interactive tests on Windows.
#[cfg(not(target_os = "windows"))]
pub fn run_sysand_interactive_in<'a, I: IntoIterator<Item = &'a str>>(
    cwd: &Path,
    args: I,
    timeout_ms: Option<u64>,
    cfg: Option<&str>,
) -> Result<PtySession, Box<dyn Error>> {
    let cmd = sysand_cmd_in(cwd, args, cfg)?;

    Ok(spawn_command(cmd, timeout_ms)?)
}

// TODO: Figure out how to do interactive tests on Windows.
#[cfg(not(target_os = "windows"))]
pub fn run_sysand_interactive<'a, I: IntoIterator<Item = &'a str>>(
    args: I,
    timeout_ms: Option<u64>,
    cfg: Option<&str>,
) -> Result<(TempDir, PathBuf, PtySession), Box<dyn Error>> {
    let (temp_dir, cwd, cmd) = sysand_cmd(args, cfg)?;

    Ok((temp_dir, cwd, spawn_command(cmd, timeout_ms)?))
}

// TODO: Figure out how to do interactive tests on Windows.
#[cfg(not(target_os = "windows"))]
pub fn await_exit(p: PtySession) -> Result<std::process::ExitStatus, Box<dyn Error>> {
    let status = p.process.wait()?;
    if let rexpect::process::wait::WaitStatus::Exited(_, code) = status {
        Ok(std::process::ExitStatus::from_raw(code))
    } else {
        Err("Failed to get exit status code".into())
    }
}
