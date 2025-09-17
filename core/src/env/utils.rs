// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::project::{ProjectMut, ProjectRead};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum CloneError<ProjectReadError, EnvironmentWriteError> {
    #[error("project read error: {0}")]
    ReadError(ProjectReadError),
    #[error("environment write error")]
    WriteError(EnvironmentWriteError),
    #[error("incomplete source")]
    IncompleteSourceError(String),
    #[error("{0}")]
    IOError(#[from] std::io::Error),
}

pub fn clone_project<P: ProjectRead, Q: ProjectMut>(
    from: &P,
    to: &mut Q,
    overwrite: bool,
) -> Result<(), CloneError<P::Error, Q::Error>> {
    match from.get_project().map_err(CloneError::ReadError)? {
        (None, None) => {
            return Err(CloneError::IncompleteSourceError(
                "missing .project.json and .meta.json".to_string(),
            ));
        }
        (None, _) => {
            return Err(CloneError::IncompleteSourceError(
                "missing .project.json".to_string(),
            ));
        }
        (_, None) => {
            return Err(CloneError::IncompleteSourceError(
                "missing .meta.json".to_string(),
            ));
        }
        (Some(info), Some(meta)) => {
            to.put_project(&info, &meta, overwrite)
                .map_err(CloneError::WriteError)?;

            for source_path in &meta.source_paths(true) {
                let mut source = from
                    .read_source(source_path)
                    .map_err(CloneError::ReadError)?;
                to.write_source(source_path, &mut source, overwrite)
                    .map_err(CloneError::WriteError)?;
            }
        }
    }

    Ok(())
}

// pub fn clone_project_into_unnormalised<P : ProjectRead, E : WriteEnvironment, S : AsRef<str>, T: AsRef<str>>(
//     project : &P,
//     environment : &mut E,
//     uri : S,
//     version : T,
//     overwrite : bool,
// ) -> Result<E::InterchangeProjectWrite, CloneError<P::ReadError, E::WriteError>> {
//     environment.put_project(
//         uri,
//         version,
//         |target| {
//             match project.get_project()? {
//                     (None, None) => todo!(),
//                     (None, _) => todo!(),
//                     (_, None) => todo!(),
//                     (Some(info), Some(meta)) => {
//                         target.put_project(&info, &meta, overwrite)?;
//                         Ok(())
//                     },
//                 }
//         }
//     ).map_err(|err: PutProjectError<E::WriteError, P::ReadError>| match err {

//     })
// }

// pub fn clone_project_into_normalised<P : ProjectRead, E : WriteEnvironment>(
//     project : &P,
//     environment : &mut E,
//     uri : Uri<String>,
//     version : Version,
//     overwrite : bool,
// ) -> Result<E::InterchangeProjectWrite, CloneError<P::ReadError, E::WriteError>> {
//     let nfc = icu_normalizer::ComposingNormalizerBorrowed::new_nfc();
//     let uri_str = uri.normalize();
//     let uri_normalised =
//         nfc.normalize(uri_str.as_str());

//     clone_project_into_unnormalised(
//         project,environment,
//         uri_normalised,
//         version.to_string(),
//         overwrite,
//     )
// }
