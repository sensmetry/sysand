// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

#[cfg(feature = "filesystem")]
use camino::Utf8PathBuf;
use semver::Version;

use crate::{
    env::utils::ErrorBound,
    manifest::{
        ModelToml,
        common::{InheritFrom, Sources},
        edit::{DocumentMut, render_new},
        inherit::{Inherited, MetaProperty},
        project::{ProjectManifest, ProjectTable},
        workspace_config::{GroupProperties, WorkspaceConfig},
    },
    model::{
        IdentifierError, IndexName, IndexPublisher, InterchangeProjectInfoRaw,
        InterchangeProjectMetadata, KnownMetamodel, Metamodel, MetamodelFamily, ProjectFieldError,
        ProjectName, ProjectPublisher,
    },
    project::{
        ManifestWriteError, ProjectMut, ProjectRead as _,
        memory::{InMemoryError, InMemoryProject},
    },
};
use typed_path::Utf8UnixPathBuf;

#[cfg(feature = "filesystem")]
use crate::project::local_src::{LocalSrcError, LocalSrcProject};

use thiserror::Error;

/// The version of a project created without one.
pub const DEFAULT_VERSION: &str = "0.0.1";

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
    /// The publisher or name is not one a `.project.json` can hold.
    #[error("invalid project identifier `{0}`")]
    ProjectField(Box<str>, #[source] ProjectFieldError),
    #[error(transparent)]
    TomlWrite(ManifestWriteError<ProjectError>),
    /// The metamodel is not one of the recognized values.
    #[error("`{0}` is not a metamodel Sysand recognizes;\nit is one of {list}", list = KnownMetamodel::RECOGNIZED)]
    Metamodel(Box<str>),
    /// A required key is neither given nor given by any workspace group.
    #[error("`{0}` is required, and no workspace group gives it")]
    Missing(RequiredKey),
    /// A key left out is given by more than one workspace group, so
    /// which one it inherits from is ambiguous.
    #[error(
        "`{key}` is given by more than one workspace group ({list}),\n\
         so the group to inherit it from is ambiguous",
        list = groups.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    AmbiguousGroup {
        key: InheritableKey,
        groups: Vec<InheritFrom>,
    },
    /// A `.meta.json` has no field to hold a Syside project's `root`.
    #[error("a `.project.json` project cannot use the Syside metamodel `{0}`")]
    JsonSysideMetamodel(Box<str>),
}

/// A `[project]` key a new project cannot do without.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequiredKey {
    Publisher,
    Metamodel,
}

impl std::fmt::Display for RequiredKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Publisher => "publisher",
            Self::Metamodel => "metamodel",
        })
    }
}

/// A `[project]` key a new project inherits from a workspace group when
/// it is left out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InheritableKey {
    Publisher,
    Version,
    License,
    Metamodel,
}

impl std::fmt::Display for InheritableKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Publisher => "publisher",
            Self::Version => "version",
            Self::License => "license",
            Self::Metamodel => "metamodel",
        })
    }
}

/// What a new project is stored as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectKind {
    /// A `model.toml`.
    ModelToml,
    /// A `.project.json` and a `.meta.json`.
    ProjectJson,
}

/// The `[project]` keys [`do_init_toml`] writes.
///
/// A key left out is inherited from the one group of
/// [`InitProject::workspace`] that gives it, the default group or a named
/// one, and written as `<key>.workspace = true` or
/// `<key>.workspace = "<group-name>"`. More than one group giving it is
/// an error. When no group gives it, `version` is [`DEFAULT_VERSION`],
/// `license` is left out, and a missing `publisher` or `metamodel` is an
/// error.
#[derive(Clone, Debug)]
pub struct InitProject<'a> {
    pub name: ProjectName,
    pub publisher: Option<ProjectPublisher>,
    pub version: Option<Version>,
    pub license: Option<spdx::Expression>,
    pub metamodel: Option<KnownMetamodel>,
    /// The workspace the project is a member of.
    pub workspace: Option<&'a WorkspaceConfig>,
}

/// A project outside any workspace that gives every required key.
#[derive(Clone, Debug)]
pub struct GivenProject {
    pub name: ProjectName,
    pub publisher: ProjectPublisher,
    pub version: Version,
    pub license: Option<spdx::Expression>,
    pub metamodel: KnownMetamodel,
}

impl GivenProject {
    /// Parse each key, the metamodel first, then `publisher`, `name`,
    /// `version` and `license`.
    pub fn parse<E: ErrorBound>(
        name: String,
        publisher: String,
        version: String,
        license: Option<String>,
        metamodel: String,
    ) -> Result<Self, InitError<E>> {
        let metamodel = recognize(metamodel)?;
        let publisher = ProjectPublisher::parse(publisher)
            .map_err(|(value, e)| InitError::ProjectField(value.into(), e))?;
        let name = ProjectName::parse(name)
            .map_err(|(value, e)| InitError::ProjectField(value.into(), e))?;
        let version =
            Version::parse(&version).map_err(|e| InitError::SemVerParse(version.into(), e))?;
        let license = license
            .map(|raw| {
                spdx::Expression::parse(&raw)
                    .map_err(|e| InitError::SPDXLicenseParse(raw.into(), e))
            })
            .transpose()?;
        Ok(Self {
            name,
            publisher,
            version,
            license,
            metamodel,
        })
    }
}

impl From<GivenProject> for InitProject<'_> {
    fn from(given: GivenProject) -> Self {
        Self {
            name: given.name,
            publisher: Some(given.publisher),
            version: Some(given.version),
            license: given.license,
            metamodel: Some(given.metamodel),
            workspace: None,
        }
    }
}

/// Create a project, a `model.toml`.
///
/// A project created with a Syside metamodel gets `root` naming one file,
/// and the file itself, created with a package declaration when it does
/// not exist. An inherited `metamodel` takes the layout of the group's
/// metamodel.
///
/// Errors if a key left out is given by more than one group
/// ([`InitError::AmbiguousGroup`]) or a required key is neither given nor
/// inherited ([`InitError::Missing`]), checked for `metamodel`, then
/// `publisher`, `version` and `license`; or if the publisher or name is
/// one a `model.toml` refuses ([`InitError::Identifier`]).
///
/// The file's existence is checked first, then the manifest is written
/// (refused when one exists), then the file.
pub fn do_init_toml<P: ProjectMut>(
    init: InitProject<'_>,
    storage: &mut P,
) -> Result<(), InitError<P::Error>> {
    plan_init_toml(init)?.write(storage)
}

/// A `model.toml` project that is ready to be written.
#[derive(Debug)]
pub struct InitPlan {
    document: DocumentMut,
    name: IndexName,
    syside_root: Option<Utf8UnixPathBuf>,
}

impl InitPlan {
    /// The text of the project's `model.toml`.
    pub fn text(&self) -> String {
        self.document.to_string()
    }

    /// Write the `model.toml`, and the root source of a Syside project that
    /// has none.
    pub fn write<P: ProjectMut>(self, storage: &mut P) -> Result<(), InitError<P::Error>> {
        let root_exists = self
            .syside_root
            .as_ref()
            .is_some_and(|root| storage.read_source(root).is_ok());
        storage
            .put_manifest_document(&self.document, false)
            .map_err(InitError::TomlWrite)?;

        if let Some(root) = self.syside_root
            && !root_exists
        {
            let declaration = format!("package {};\n", package_name(&self.name.normalized()));
            storage
                .write_source(&root, &mut declaration.as_bytes(), false)
                .map_err(InitError::Project)?;
        }
        Ok(())
    }
}

/// Decide what [`do_init_toml`] would write, without writing it.
pub fn plan_init_toml<E: ErrorBound>(init: InitProject<'_>) -> Result<InitPlan, InitError<E>> {
    let InitProject {
        name,
        publisher,
        version,
        license,
        metamodel,
        workspace,
    } = init;

    // The metamodel the project ends up with decides the layout.
    let (metamodel, kind) = if let Some(value) = metamodel {
        let kind = value.clone();
        (MetaProperty::Provided(value), kind)
    } else {
        let (inherited, kind) = sole_group(InheritableKey::Metamodel, workspace, |group| {
            group.metamodel.as_ref()
        })?
        .ok_or(InitError::Missing(RequiredKey::Metamodel))?;
        (MetaProperty::Inherited(inherited), kind.clone())
    };
    let publisher = publisher
        .map(|publisher| {
            IndexPublisher::parse(publisher.into_string()).map_err(InitError::Identifier)
        })
        .transpose()?;
    let publisher = given_or_inherited(publisher, InheritableKey::Publisher, workspace, |group| {
        group.publisher.as_ref()
    })?
    .ok_or(InitError::Missing(RequiredKey::Publisher))?;
    let name = IndexName::parse(name.into_string()).map_err(InitError::Identifier)?;
    let version = given_or_inherited(version, InheritableKey::Version, workspace, |group| {
        group.version.as_ref()
    })?
    .unwrap_or_else(|| MetaProperty::Provided(default_version()));
    let license = given_or_inherited(license, InheritableKey::License, workspace, |group| {
        group.license.as_ref()
    })?;

    log_creating(&name);

    let syside_root = (kind.family == MetamodelFamily::Syside)
        .then(|| Utf8UnixPathBuf::from(format!("{}{}", name.normalized(), kind.spec.file_ext())));
    let mut project = ProjectTable::new(publisher, name.clone(), version);
    match &syside_root {
        Some(root) => project.root = Some(toml::Value::from(root.as_str())),
        // Written as `sources = []`: an absent key is a manifest that
        // does not resolve.
        None => project.sources = Some(Sources::default()),
    }
    project.license = license;
    project.metamodel = Some(metamodel);

    let manifest = ModelToml::Project(Box::new(ProjectManifest {
        project,
        inline_usages: vec![],
    }));

    Ok(InitPlan {
        document: render_new(&manifest),
        name,
        syside_root,
    })
}

/// Create a project stored as `.project.json` and `.meta.json`.
///
/// The metamodel must be a recognized OMG one.
pub fn do_init_json<P: ProjectMut>(
    project: GivenProject,
    storage: &mut P,
) -> Result<(), InitError<P::Error>> {
    let GivenProject {
        name,
        publisher,
        version,
        license,
        metamodel,
    } = project;
    if metamodel.family == MetamodelFamily::Syside {
        return Err(InitError::JsonSysideMetamodel(metamodel.to_string().into()));
    }

    log_creating(&name);

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

fn recognize<E: ErrorBound>(value: String) -> Result<KnownMetamodel, InitError<E>> {
    KnownMetamodel::recognize(&value).ok_or_else(|| InitError::Metamodel(value.into()))
}

/// [`DEFAULT_VERSION`], parsed.
pub fn default_version() -> Version {
    Version::parse(DEFAULT_VERSION).expect("the default version is a SemVer")
}

fn log_creating(name: &impl std::fmt::Display) {
    let creating = "Creating";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{creating:>12}{header:#} interchange project `{name}`");
}

/// A value given directly, else the marker naming the one workspace
/// group that gives the key, as `pick` reads it out of a group.
///
/// Errors if more than one group gives the key.
fn given_or_inherited<T, U, E: ErrorBound>(
    given: Option<T>,
    key: InheritableKey,
    workspace: Option<&WorkspaceConfig>,
    pick: impl Fn(&GroupProperties) -> Option<&U>,
) -> Result<Option<MetaProperty<T>>, InitError<E>> {
    Ok(match given {
        Some(value) => Some(MetaProperty::Provided(value)),
        None => sole_group(key, workspace, pick)?
            .map(|(inherited, _)| MetaProperty::Inherited(inherited)),
    })
}

/// The marker naming the one workspace group that gives the key, as
/// `pick` reads it out of a group, and the value the group gives.
///
/// Errors if more than one group gives the key.
fn sole_group<'a, U, E: ErrorBound>(
    key: InheritableKey,
    workspace: Option<&'a WorkspaceConfig>,
    pick: impl Fn(&'a GroupProperties) -> Option<&'a U>,
) -> Result<Option<(Inherited, &'a U)>, InitError<E>> {
    let mut giving = workspace
        .into_iter()
        .flat_map(WorkspaceConfig::all_groups)
        .filter_map(|(from, group)| pick(group).map(|value| (from, value)));
    let Some((from, value)) = giving.next() else {
        return Ok(None);
    };
    let others: Vec<InheritFrom> = giving.map(|(from, _)| from).collect();
    if !others.is_empty() {
        let groups = std::iter::once(from).chain(others).collect();
        return Err(InitError::AmbiguousGroup { key, groups });
    }
    Ok(Some((Inherited { from, at: None }, value)))
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
    metamodel: String,
) -> Result<InMemoryProject, InitError<InMemoryError>> {
    let given = GivenProject::parse(
        name.as_ref().to_owned(),
        publisher.as_ref().to_owned(),
        version.as_ref().to_owned(),
        license,
        metamodel,
    )?;
    let mut storage = InMemoryProject::default();
    do_init_toml(given.into(), &mut storage)?;
    Ok(storage)
}

#[cfg(feature = "filesystem")]
pub fn do_init_local_file(
    name: String,
    publisher: String,
    version: String,
    license: Option<String>,
    metamodel: String,
    kind: ProjectKind,
    path: Utf8PathBuf,
) -> Result<LocalSrcProject, InitError<LocalSrcError>> {
    let given = GivenProject::parse(name, publisher, version, license, metamodel)?;
    let mut storage = LocalSrcProject::new_access(path, None);
    match kind {
        ProjectKind::ModelToml => do_init_toml(given.into(), &mut storage)?,
        ProjectKind::ProjectJson => do_init_json(given, &mut storage)?,
    }

    Ok(storage)
}

#[cfg(test)]
#[path = "./init_tests.rs"]
mod tests;
