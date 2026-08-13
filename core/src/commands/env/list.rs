// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use crate::{env::ReadEnvironment, utils::format_err};

pub fn do_env_list<E: ReadEnvironment>(
    env: &E,
) -> Result<Vec<(String, Option<String>)>, E::ReadError> {
    let uris = env.uris()?.into_iter().filter_map(|res| match res {
        Ok(u) => Some(u),
        Err(e) => {
            log::warn!("failed to read uri: {}", format_err(e));
            None
        }
    });

    let mut result = Vec::new();
    for uri in uris {
        let mut versions = env
            .versions(&uri)?
            .into_iter()
            .filter_map(|res| match res {
                Ok(u) => Some(u),
                Err(e) => {
                    log::warn!("failed to read one version of `{uri}`: {}", format_err(e));
                    None
                }
            })
            .peekable();

        if versions.peek().is_none() {
            result.push((uri, None));
        } else {
            result.extend(versions.map(|v| (uri.clone(), Some(v))));
        }
    }

    Ok(result)
}
