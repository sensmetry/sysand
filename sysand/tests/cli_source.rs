// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use assert_cmd::prelude::*;
//use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;
use predicates::prelude::PredicateBooleanExt;

#[test]
fn list_sources() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir1, cwd_dep, out) =
        run_sysand(["new", "--version", "1.2.3", "list_sources_dep"], None)?;
    out.assert().success();

    let dep_path = cwd_dep.join("list_sources_dep");

    std::fs::write(dep_path.join("dep_src.sysml"), "package DepSrc;")?;

    let out = run_sysand_in(&dep_path, ["include", "dep_src.sysml"], None)?;
    out.assert().success();

    let (_temp_dir2, cwd, out) = run_sysand(["new", "--version", "1.2.3", "list_sources"], None)?;
    out.assert().success();

    let path = cwd.join("list_sources");

    std::fs::write(path.join("src.sysml"), "package Src;")?;

    let out = run_sysand_in(&path, ["include", "src.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(
        &path,
        [
            "env",
            "install",
            "urn:kpar:list_sources_dep",
            "--location",
            dep_path.to_str().unwrap(),
        ],
        None,
    )?;
    out.assert().success();

    let out = run_sysand_in(
        &path,
        [
            "add",
            "--no-sync",
            "urn:kpar:list_sources_dep",
            "--no-index",
            "--verbose",
        ],
        None,
    )?;
    out.assert().success();

    let mut expected_path = path.join("src.sysml").to_str().unwrap().to_string();
    expected_path.push('\n');
    let mut dep_expected_path = path
        .join("sysand_env")
        .join("585221b9a7b5e0baeeb2c12946f85975f843982d15e7aba9bcf712c83a4a9be9")
        .join("1.2.3.kpar")
        .join("dep_src.sysml")
        .to_str()
        .unwrap()
        .to_string();
    dep_expected_path.push('\n');
    let mut combined_path = "".to_string();
    combined_path.push_str(&expected_path);
    combined_path.push_str(&dep_expected_path);

    let out = run_sysand_in(&path, ["sources", "--no-deps"], None)?;

    out.assert().success().stdout(expected_path);

    let out = run_sysand_in(
        &path,
        ["env", "sources", "urn:kpar:list_sources_dep", "--no-deps"],
        None,
    )?;

    out.assert().success().stdout(dep_expected_path);

    let out = run_sysand_in(&path, ["sources"], None)?;

    out.assert().success().stdout(combined_path);

    Ok(())
}

#[test]
fn sources_without_std() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir1, cwd_dep, out) = run_sysand(
        ["new", "--version", "1.2.3", "sources_without_std_dep"],
        None,
    )?;
    out.assert().success();

    let path_dep = cwd_dep.join("sources_without_std_dep");

    std::fs::write(path_dep.join("src_dep.sysml"), "package SrcDep;")?;

    let out = run_sysand_in(&path_dep, ["include", "src_dep.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(
        &path_dep,
        ["add", "--no-index", "urn:kpar:function-library"],
        None,
    )?;
    out.assert().success();

    let out = run_sysand_in(
        &path_dep,
        ["add", "--no-index", "urn:kpar:function-library"],
        None,
    )?;
    out.assert().success();

    let out = run_sysand_in(
        &path_dep,
        ["add", "--no-index", "urn:kpar:function-library"],
        None,
    )?;
    out.assert().success();

    let out = run_sysand_in(
        &path_dep,
        [
            "add",
            "--no-index",
            "https://www.omg.org/spec/KerML/20230201/Function-Library.kpar",
        ],
        None,
    )?;
    out.assert().success();

    let out = run_sysand_in(
        &path_dep,
        [
            "add",
            "--no-index",
            "https://www.omg.org/spec/KerML/20250201/Function-Library.kpar",
        ],
        None,
    )?;
    out.assert().success();

    let (_temp_dir2, cwd, out) =
        run_sysand(["new", "--version", "1.2.3", "sources_without_std"], None)?;
    out.assert().success();

    let path = cwd.join("sources_without_std");

    std::fs::write(path.join("src.sysml"), "package Src;")?;

    let out = run_sysand_in(&path, ["include", "src.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&path, ["env"], None)?;
    out.assert().success();

    let out = run_sysand_in(
        &path,
        [
            "env",
            "install",
            "urn:kpar:sources_without_std_dep",
            "--location",
            path_dep.to_str().unwrap(),
        ],
        None,
    )?;
    out.assert().success();

    let out = run_sysand_in(
        &path,
        ["add", "--no-index", "urn:kpar:sources_without_std_dep"],
        None,
    )?;
    out.assert().success();

    let out = run_sysand_in(&path, ["sources"], None)?;
    out.assert().success().stdout(
        predicates::str::contains("src.sysml").and(predicates::str::contains("src_dep.sysml")),
    );

    Ok(())
}
