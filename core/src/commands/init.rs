// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

#[cfg(feature = "filesystem")]
use camino::Utf8PathBuf;
use semver::Version;
use spdx;

use crate::{
    env::utils::ErrorBound,
    manifest::{
        ModelToml,
        common::Sources,
        edit::render_new,
        inherit::MetaProperty,
        project::{ProjectManifest, ProjectTable},
    },
    model::{IdentifierError, IndexName, IndexPublisher, KnownMetamodel},
    project::{ManifestWriteError, ProjectMut, memory::InMemoryProject},
};

#[cfg(feature = "filesystem")]
use crate::project::local_src::{LocalSrcError, LocalSrcProject};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum InitError<ProjectError: ErrorBound> {
    #[error("failed to parse `{0}` as a Semantic Version: {1}")]
    SemVerParse(Box<str>, semver::Error),
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error("failed to parse `{0}` as an SPDX license expression:\n{1}")]
    SPDXLicenseParse(Box<str>, spdx::error::ParseError),
    /// The publisher or name is not one a `model.toml` can hold.
    ///
    /// Not a check `init` chose to make: a manifest holds the validated
    /// types, so a project that cannot be named cannot be created. The
    /// frontend adds where to put the fix -- the name usually comes from
    /// the directory rather than from an argument.
    #[error("`{value}`: {error}", value = .0.value, error = .0)]
    Identifier(IdentifierError),
    #[error(transparent)]
    Write(ManifestWriteError<ProjectError>),
    /// model-toml.md "Metamodel": the value set is closed, so a project
    /// cannot be created with one outside it. The CLI checks its own
    /// argument earlier, where the message can name the flag; this is
    /// what a binding caller gets.
    #[error("`{0}` is not a metamodel Sysand recognizes;\nit is one of {list}", list = KnownMetamodel::RECOGNIZED)]
    Metamodel(Box<str>),
}

/// Create a project.
///
/// model-toml.md: "`sysand init`, `sysand new` and `sysand migrate`
/// write the current value" of `manifest-version` -- so what a new
/// project is, is a `model.toml`. The two json documents are what a
/// build produces and what a legacy project has.
///
/// The three identifiers are parsed rather than checked: a
/// [`ProjectTable`] holds the validated types, so parsing them is the
/// only way to build one. A publisher or name the rule refuses is
/// [`InitError::Identifier`].
/// The metamodel, when given, must be one of the four a `model.toml`
/// recognizes ([`InitError::Metamodel`]).
pub fn do_init<P: ProjectMut>(
    name: String,
    publisher: String,
    version: String,
    license: Option<String>,
    metamodel: Option<MetaProperty<String>>,
    storage: &mut P,
) -> Result<(), InitError<P::Error>> {
    let version =
        Version::parse(&version).map_err(|e| InitError::SemVerParse(version.as_str().into(), e))?;
    let publisher = IndexPublisher::parse(publisher).map_err(InitError::Identifier)?;
    let name = IndexName::parse(name).map_err(InitError::Identifier)?;
    let license = license
        .map(|raw| {
            spdx::Expression::parse(&raw)
                .map_err(|e| InitError::SPDXLicenseParse(raw.as_str().into(), e))
        })
        .transpose()?;
    // A marker names a group, not a metamodel, so there is nothing to
    // parse: what it resolves to is the workspace's business, and the
    // reader's.
    let metamodel = metamodel
        .map(|metamodel| match metamodel {
            MetaProperty::Provided(value) => KnownMetamodel::recognize(&value)
                .map(MetaProperty::Provided)
                .ok_or_else(|| InitError::Metamodel(value.into())),
            MetaProperty::Inherited(inherited) => Ok(MetaProperty::Inherited(inherited)),
        })
        .transpose()?;

    let creating = "Creating";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{creating:>12}{header:#} interchange project `{name}`");

    let mut project = ProjectTable::new(publisher, name, version);
    // model-toml.md: "a project that has none is written as `sources =
    // []`" -- an absent key is a manifest that does not resolve.
    project.sources = Some(Sources::default());
    project.license = license.map(MetaProperty::Provided);
    project.metamodel = metamodel;

    let manifest = ModelToml::Project(Box::new(ProjectManifest {
        project,
        inline_usages: vec![],
    }));

    storage
        .put_manifest_document(&render_new(&manifest), false)
        .map_err(InitError::Write)
}

pub fn do_init_memory<N: AsRef<str>, P: AsRef<str>, V: AsRef<str>>(
    name: N,
    publisher: P,
    version: V,
    license: Option<String>,
    metamodel: Option<MetaProperty<String>>,
) -> Result<InMemoryProject, InitError<crate::project::memory::InMemoryError>> {
    let mut storage = InMemoryProject::default();

    do_init(
        name.as_ref().to_owned(),
        publisher.as_ref().to_owned(),
        version.as_ref().to_owned(),
        license,
        metamodel,
        &mut storage,
    )?;

    Ok(storage)
}

#[cfg(feature = "filesystem")]
pub fn do_init_local_file(
    name: String,
    publisher: String,
    version: String,
    license: Option<String>,
    metamodel: Option<MetaProperty<String>>,
    path: Utf8PathBuf,
) -> Result<LocalSrcProject, InitError<LocalSrcError>> {
    let mut storage = LocalSrcProject::new_access(path, None);
    do_init(name, publisher, version, license, metamodel, &mut storage)?;

    Ok(storage)
}

#[cfg(test)]
#[path = "./init_tests.rs"]
mod tests;
