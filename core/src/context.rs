#[cfg(feature = "filesystem")]
use crate::{project::local_src::LocalSrcProject, workspace::Workspace};

#[derive(Debug, Default)]
pub struct ProjectContext {
    /// Root directory of current workspace
    #[cfg(feature = "filesystem")]
    pub current_workspace: Option<Workspace>,
    /// Root directory of current project
    #[cfg(feature = "filesystem")]
    pub current_project: Option<LocalSrcProject>,
}
