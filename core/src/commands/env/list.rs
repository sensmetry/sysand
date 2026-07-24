// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use crate::{env::ReadEnvironment, utils::format_err};

pub fn do_env_list<E: ReadEnvironment>(
    env: E,
) -> Result<Vec<(String, Option<String>)>, E::ReadError> {
    let uris: Vec<String> = env
        .uris()?
        .into_iter()
        .filter_map(|res| match res {
            Ok(u) => Some(u),
            Err(e) => {
                log::warn!("failed to read uri: {}", format_err(e));
                None
            }
        })
        .collect();

    #[allow(clippy::type_complexity)]
    let nested: Result<Vec<Vec<(String, Option<String>)>>, E::ReadError> = uris
        .into_iter()
        .map(|uri| {
            let versions: Vec<String> = env
                .versions(&uri)?
                .into_iter()
                .filter_map(|res| match res {
                    Ok(u) => Some(u),
                    Err(e) => {
                        log::warn!("failed to read one version of `{uri}`: {}", format_err(e));
                        None
                    }
                })
                .collect();

            if versions.is_empty() {
                Ok(vec![(uri, None)])
            } else {
                Ok(versions
                    .into_iter()
                    .map(|v| (uri.clone(), Some(v)))
                    .collect())
            }
        })
        .collect();

    nested.map(|v| v.into_iter().flatten().collect())
}
