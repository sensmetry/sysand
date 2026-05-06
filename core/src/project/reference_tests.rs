// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::project::{local_kpar::LocalKParProject, reference::ProjectReference};
#[test]
fn project_reference_is_cloneable() {
    let kpar = ProjectReference::new(LocalKParProject::new("path", "root").unwrap());
    let _clone = kpar.clone();
}
