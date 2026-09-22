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
    model::{
        IdentifierError, IndexName, IndexPublisher, KnownMetamodel, MetamodelFamily, SpecKind,
    },
    project::{ManifestWriteError, ProjectMut, ProjectRead as _, memory::InMemoryProject},
};
use typed_path::Utf8UnixPathBuf;

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
    do_init_ext(name, publisher, version, license, metamodel, None, storage)
}

/// [`do_init`], told which metamodel an inherited marker resolves to.
///
/// design/syside-metamodel.md, "Commands": a project created with a
/// Syside metamodel gets the Syside layout -- `sources` listing one
/// file, `root` naming it, and the file itself, created with a package
/// declaration when it does not exist. A marker names a group, not a
/// metamodel, so the family behind it is the caller's to supply
/// (`inherited_kind`); without it, the layout is the OMG one.
///
/// The file is created rather than merely named so that a generated project
/// never lists a file that is not there. Order: the file's existence is
/// checked, the manifest is written (and refused, before anything else,
/// when one exists), then the file.
pub fn do_init_ext<P: ProjectMut>(
    name: String,
    publisher: String,
    version: String,
    license: Option<String>,
    metamodel: Option<MetaProperty<String>>,
    inherited_kind: Option<KnownMetamodel>,
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

    let kind = match &metamodel {
        Some(MetaProperty::Provided(value)) => Some(value.clone()),
        Some(MetaProperty::Inherited(_)) => inherited_kind,
        None => None,
    };
    let syside_root = kind
        .filter(|kind| kind.family == MetamodelFamily::Syside)
        .map(|kind| {
            let extension = match kind.spec {
                SpecKind::KerMl => "kerml",
                SpecKind::SysMl => "sysml",
            };
            Utf8UnixPathBuf::from(format!("{}.{extension}", name.normalized()))
        });
    let root_exists = syside_root
        .as_ref()
        .is_some_and(|root| storage.read_source(root).is_ok());

    let mut project = ProjectTable::new(publisher, name.clone(), version);
    match &syside_root {
        Some(root) => {
            project.sources = Some(
                Sources::new(vec![root.clone()])
                    .expect("a normalized name with a model extension is a `sources` entry"),
            );
            project.root = Some(toml::Value::from(root.as_str()));
        }
        // model-toml.md: "a project that has none is written as `sources =
        // []`" -- an absent key is a manifest that does not resolve.
        None => project.sources = Some(Sources::default()),
    }
    project.license = license.map(MetaProperty::Provided);
    project.metamodel = metamodel;

    let manifest = ModelToml::Project(Box::new(ProjectManifest {
        project,
        inline_usages: vec![],
    }));

    storage
        .put_manifest_document(&render_new(&manifest), false)
        .map_err(InitError::Write)?;

    if let Some(root) = syside_root
        && !root_exists
    {
        let declaration = format!("package {};\n", package_name(&name.normalized()));
        storage
            .write_source(&root, &mut declaration.as_bytes(), false)
            .map_err(InitError::Project)?;
    }
    Ok(())
}

/// The name as a KerML/SysML package declaration writes it: bare when it
/// is an identifier, quoted (`'my-widgets'`) when the normalized name
/// carries a `-` or `.` or starts with a digit -- all of which the
/// language allows in a quoted name and none of which in a bare one.
fn package_name(stem: &str) -> String {
    let bare = stem
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if bare {
        stem.to_owned()
    } else {
        format!("'{stem}'")
    }
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
