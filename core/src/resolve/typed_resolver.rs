use camino::Utf8Path;
use url::Url;

use crate::{
    auth::HTTPAuthentication,
    env::utils::ErrorBound,
    model::{InterchangeProjectUsage, InterchangeProjectUsageG},
    project::ProjectRead,
    resolve::{
        ResolutionOutcome, ResolveRead,
        file::FileResolver,
        gix_git::GitResolver,
        reqwest_http::HTTPResolverAsync,
        standard::{RemoteIndexResolver, StandardResolver},
    },
};

// TODO: maybe adapt CombinedResolver for this use case?
/// A resolver for resolving specific dependency types:
/// - URL: schemes `file:`/`http(s)`
/// - path: relative path; this will only work for local projects, for remote projects
///         it doesn't make sense
/// - git: schemes `git:`/`ssh:`
/// - index
/// - resource: uses StandardResolver
pub struct TypedResolver<Policy: HTTPAuthentication> {
    file: FileResolver,
    http: HTTPResolverAsync<Policy>,
    git: GitResolver,
    index: RemoteIndexResolver<Policy>,
    resource: StandardResolver<Policy>,
}

// easyfind3
//
// Resolver design questions:
// - should resolvers be split; if so, how?:
//   - use unified for any usage
//   - split out resource from specific types
//   - split out every type
// - interface:
//   - modify ResolveRead to take:
//     - InterchangeProjectUsage
//     - implementor-specific type
//   - add inherent methods for each resolver, taking:
//     - specific type
//     - InterchangeProjectUsage (+ base path for path resolver)
//   - should both kpar and src be supported for specific types?
//     - resource will continue to support what it does
//     - git: currently only src
//     - http: currently supported
//     - local (file url/path): currently supported
//     - index: currently supported, new index will not support, so only kpar support is fine
//              (provided that env->index is convenient)
//   - interface style:
//     - take broad, return err if wrong type
//     - take specific (impractical to differentiate by type for resource)
//     - take broad, return ResolutionOutcome::Unresolvable for wrong type (current impl for resource)
//     - separate method for checking if supplied is acceptable - multiple calls for
//       every resolve needed
//
// What about path usages:
// - allow absolute and relative
// - match Cargo, it resolves path usages taking a base path to be the Cargo.toml that
//   declared them, not the root Cargo.toml
// - then dep resolution has to be modified to have project path (if known) for
//   each project in dependency graph, at least for the time its direct usages
//   are resolved
// - remote projects can't resolve path usages (no matter relative or absolute)
// - should both kpar and src be supported?
impl<Policy: HTTPAuthentication> TypedResolver<Policy> {
    pub fn new(
        file: FileResolver,
        http: HTTPResolverAsync<Policy>,
        git: GitResolver,
        index: RemoteIndexResolver<Policy>,
        resource: StandardResolver<Policy>,
    ) -> Self {
        Self {
            file,
            http,
            git,
            index,
            resource,
        }
    }

    pub fn resolve(
        &self,
        usage: &InterchangeProjectUsage,
        base_path: Option<impl AsRef<Utf8Path>>,
    ) -> Result<ResolutionOutcome<impl ProjectRead>, impl ErrorBound> {
        match usage {
            InterchangeProjectUsage::Resource {
                resource,
                version_constraint,
            } => self.resource.resolve_read(resource),
            InterchangeProjectUsage::Url {
                url,
                publisher,
                name,
            } => todo!(),
            InterchangeProjectUsage::Path {
                path,
                publisher,
                name,
            } => todo!(),
            InterchangeProjectUsage::Git {
                git,
                id,
                publisher,
                name,
            } => todo!(),
            InterchangeProjectUsage::Index {
                publisher,
                name,
                version_constraint,
            } => todo!(),
        }
    }
}
