// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use anyhow::Result;
use camino::Utf8Path;

use sysand_core::index::{do_index_add, do_index_init, do_index_remove, do_index_yank};

pub fn command_index_init<R: AsRef<Utf8Path>>(index_root: R) -> Result<()> {
    do_index_init(index_root)?;
    Ok(())
}

pub fn command_index_add<R: AsRef<Utf8Path>, P: AsRef<Utf8Path>, I: AsRef<str>>(
    index_root: R,
    kpar_path: P,
    iri: Option<I>,
) -> Result<()> {
    do_index_add(index_root, kpar_path, iri)?;
    Ok(())
}

pub fn command_index_yank<R: AsRef<Utf8Path>, I: AsRef<str>, V: AsRef<str>>(
    index_root: R,
    iri: I,
    version: V,
) -> Result<()> {
    do_index_yank(index_root, iri, version)?;
    Ok(())
}

pub fn command_index_remove<R: AsRef<Utf8Path>, I: AsRef<str>, V: AsRef<str>>(
    index_root: R,
    iri: I,
    version: Option<V>,
) -> Result<()> {
    do_index_remove(index_root, iri, version)?;
    Ok(())
}
