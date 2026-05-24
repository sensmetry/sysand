// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use anyhow::Result;
use camino::Utf8Path;

use sysand_core::index::{
    RemoveTarget, do_index_add, do_index_init, do_index_remove, do_index_yank,
};

pub fn command_index_init<R: AsRef<Utf8Path>>(index_root: R) -> Result<()> {
    do_index_init(index_root)?;
    Ok(())
}

pub fn command_index_add<I: AsRef<str>, P: AsRef<Utf8Path>, R: AsRef<Utf8Path>>(
    iri: Option<I>,
    kpar_path: P,
    index_root: R,
) -> Result<()> {
    do_index_add(iri, kpar_path, index_root)?;
    Ok(())
}

pub fn command_index_yank<I: AsRef<str>, V: AsRef<str>, R: AsRef<Utf8Path>>(
    iri: I,
    version: V,
    index_root: R,
) -> Result<()> {
    do_index_yank(iri, version, index_root)?;
    Ok(())
}

pub fn command_index_remove<I: AsRef<str>, R: AsRef<Utf8Path>>(
    iri: I,
    target: RemoveTarget,
    index_root: R,
) -> Result<()> {
    do_index_remove(iri, target, index_root)?;
    Ok(())
}
