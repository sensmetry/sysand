// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::{Deserialize, Serialize};

#[cfg(feature = "filesystem")]
pub mod local_fs;

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub quiet: Option<bool>,
    pub verbose: Option<bool>,
    pub index: Option<Vec<Index>>,
}

impl Config {
    pub fn merge(&mut self, config: Config) {
        self.quiet = self.quiet.or(config.quiet);
        self.verbose = self.verbose.or(config.verbose);
        extend_option_vec(&mut self.index, config.index);
    }
}

fn extend_option_vec<T>(target: &mut Option<Vec<T>>, src: Option<Vec<T>>) {
    if let Some(mut src_vec) = src {
        target.get_or_insert_with(Vec::new).append(&mut src_vec);
    }
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Index {
    name: Option<String>,
    url: String,
    explicit: Option<bool>,
    default: Option<bool>,
}

#[cfg(test)]
mod tests {
    use crate::config::{Config, Index};

    #[test]
    fn default_config() {
        let config = Config::default();

        assert_eq!(config.quiet, None);
        assert_eq!(config.verbose, None);
        assert_eq!(config.index, None);
    }

    #[test]
    fn default_index() {
        let index = Index::default();

        assert_eq!(index.name, None);
        assert_eq!(index.url, "");
        assert_eq!(index.explicit, None);
        assert_eq!(index.default, None);
    }

    #[test]
    fn merge() {
        let mut defaults = Config::default();
        let config = Config {
            quiet: Some(true),
            verbose: Some(false),
            index: Some(vec![Index {
                url: "http://www.example.com".to_string(),
                ..Default::default()
            }]),
        };
        defaults.merge(config.clone());

        assert_eq!(defaults, config);
    }
}
