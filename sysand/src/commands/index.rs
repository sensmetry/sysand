// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use anyhow::Result;
use camino::Utf8Path;

use sysand_core::index::{do_index_add, do_index_init, do_index_remove, do_index_yank};

pub fn command_index_init() -> Result<()> {
    do_index_init()?;
    Ok(())
}

pub fn command_index_add<P: AsRef<Utf8Path>, I: AsRef<str>>(
    kpar_path: P,
    iri: Option<I>,
) -> Result<()> {
    do_index_add(kpar_path, iri)?;
    Ok(())
}

pub fn command_index_yank<I: AsRef<str>, V: AsRef<str>>(iri: I, version: V) -> Result<()> {
    do_index_yank(iri, version)?;
    Ok(())
}

pub fn command_index_remove<I: AsRef<str>, V: AsRef<str>>(
    iri: I,
    version: Option<V>,
) -> Result<()> {
    do_index_remove(iri, version)?;
    Ok(())
}
