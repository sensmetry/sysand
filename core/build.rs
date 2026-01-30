// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default() == "msvc" {
        // Statically link VCRUNTIME140.dll when building for MSVC target
        // because otherwise Sysand may segfault if the system has a conflicting
        // version of VCRUNTIME140.dll.

        // Do not link these versions of VCRUNTIME140.dll
        println!("cargo:rustc-link-arg=/NODEFAULTLIB:libvcruntimed.lib");
        println!("cargo:rustc-link-arg=/NODEFAULTLIB:vcruntime.lib");
        println!("cargo:rustc-link-arg=/NODEFAULTLIB:vcruntimed.lib");
        // Link this version of VCRUNTIME140.dll
        println!("cargo:rustc-link-arg=/DEFAULTLIB:libvcruntime.lib");
        return Ok(());
    }

    Ok(())
}
