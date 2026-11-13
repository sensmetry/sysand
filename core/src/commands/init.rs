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
        IdentifierError, IndexName, IndexPublisher, InterchangeProjectInfoRaw,
        InterchangeProjectMetadata, KnownMetamodel, Metamodel, MetamodelFamily,
    },
    project::{ManifestWriteError, ProjectMut, ProjectRead as _, memory::InMemoryProject},
};
use typed_path::Utf8UnixPathBuf;

#[cfg(feature = "filesystem")]
use crate::project::local_src::{LocalSrcError, LocalSrcProject};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum InitError<ProjectError: ErrorBound> {
    #[error("failed to parse `{0}` as a Semantic Version")]
    SemVerParse(Box<str>, #[source] semver::Error),
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error("failed to parse `{0}` as an SPDX license expression:\n{1}")]
    SPDXLicenseParse(Box<str>, spdx::error::ParseError),
    /// The publisher or name is not one a `model.toml` can hold.
    #[error("invalid project identifier `{}`", .0.value)]
    Identifier(#[source] IdentifierError),
    #[error(transparent)]
    TomlWrite(ManifestWriteError<ProjectError>),
    /// The metamodel is not one of the recognized values.
    #[error("`{0}` is not a metamodel Sysand recognizes;\nit is one of {list}", list = KnownMetamodel::RECOGNIZED)]
    Metamodel(Box<str>),
    /// A `.meta.json` project declares a metamodel.
    #[error("a `.project.json` project requires a metamodel")]
    JsonMissingMetamodel,
    /// A `.meta.json` has no field to hold a marker.
    #[error("a `.project.json` project cannot inherit its metamodel from a workspace")]
    JsonInheritedMetamodel,
    /// A `.meta.json` has no field to hold a Syside project's `root`.
    #[error("a `.project.json` project cannot use the Syside metamodel `{0}`")]
    JsonSysideMetamodel(Box<str>),
}

/// What a new project is stored as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectKind {
    /// A `model.toml`.
    ModelToml,
    /// A `.project.json` and a `.meta.json`.
    ProjectJson,
}

/// Create a project.
///
/// A new project is a `model.toml`.
///
/// Errors if the publisher or name is refused ([`InitError::Identifier`]) or
/// the metamodel, when given, is not one a `model.toml` recognizes
/// ([`InitError::Metamodel`]).
pub fn do_init_toml<P: ProjectMut>(
    name: String,
    publisher: String,
    version: String,
    license: Option<String>,
    metamodel: Option<MetaProperty<String>>,
    storage: &mut P,
) -> Result<(), InitError<P::Error>> {
    do_init_toml_ext(name, publisher, version, license, metamodel, None, storage)
}

/// [`do_init_toml`], told which metamodel an inherited marker resolves to.
///
/// A project created with a Syside metamodel gets `sources` listing one
/// file, `root` naming it, and the file itself, created with a package
/// declaration when it does not exist. A marker names a group, not a
/// metamodel, so the caller supplies the metamodel it resolves to as
/// `inherited_kind`; without it, the layout is the OMG one.
///
/// The file's existence is checked first, then the manifest is written
/// (refused when one exists), then the file.
pub fn do_init_toml_ext<P: ProjectMut>(
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
    // A marker names a group, so there is nothing to parse.
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
            Utf8UnixPathBuf::from(format!("{}{}", name.normalized(), kind.spec.file_ext()))
        });
    let root_exists = syside_root
        .as_ref()
        .is_some_and(|root| storage.read_source(root).is_ok());

    let mut project = ProjectTable::new(publisher, name.clone(), version);
    match &syside_root {
        Some(root) => project.root = Some(toml::Value::from(root.as_str())),
        // Written as `sources = []`: an absent key is a manifest that
        // does not resolve.
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
        .map_err(InitError::TomlWrite)?;

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

/// Create a project stored as `.project.json` and `.meta.json`.
///
/// The metamodel is required and must be a recognized OMG one.
pub fn do_init_json<P: ProjectMut>(
    name: String,
    publisher: String,
    version: String,
    license: Option<String>,
    metamodel: MetaProperty<String>,
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
    let metamodel = match metamodel {
        MetaProperty::Provided(value) => {
            KnownMetamodel::recognize(&value).ok_or_else(|| InitError::Metamodel(value.into()))?
        }
        MetaProperty::Inherited(_) => return Err(InitError::JsonInheritedMetamodel),
    };
    if metamodel.family == MetamodelFamily::Syside {
        return Err(InitError::JsonSysideMetamodel(metamodel.to_string().into()));
    }

    let creating = "Creating";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{creating:>12}{header:#} interchange project `{name}`");

    storage
        .put_project(
            &InterchangeProjectInfoRaw {
                name: name.into_string(),
                publisher: Some(publisher.into_string()),
                description: None,
                version: version.to_string(),
                license: license.map(|l| l.to_string()),
                maintainer: vec![],
                topic: vec![],
                usage: vec![],
                website: None,
            },
            &InterchangeProjectMetadata {
                index: Some(indexmap::IndexMap::new()),
                root: None,
                exports: None,
                created: jiff::Timestamp::now(),
                metamodel: Some(Metamodel::Known(metamodel)),
                includes_derived: None,
                includes_implied: None,
                checksum: None,
            }
            .into(),
            false,
        )
        .map_err(InitError::Project)
}

/// The name as a KerML/SysML package declaration writes it: bare when it
/// is an identifier, quoted (`'my-widgets'`) when the normalized name
/// carries a `-` or `.` or starts with a digit.
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

    do_init_toml(
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
    kind: ProjectKind,
    path: Utf8PathBuf,
) -> Result<LocalSrcProject, InitError<LocalSrcError>> {
    let mut storage = LocalSrcProject::new_access(path, None);
    match kind {
        ProjectKind::ModelToml => {
            do_init_toml(name, publisher, version, license, metamodel, &mut storage)?;
        }
        ProjectKind::ProjectJson => {
            let metamodel = metamodel.ok_or(InitError::JsonMissingMetamodel)?;
            do_init_json(name, publisher, version, license, metamodel, &mut storage)?;
        }
    }

    Ok(storage)
}

#[cfg(test)]
#[path = "./init_tests.rs"]
mod tests;
