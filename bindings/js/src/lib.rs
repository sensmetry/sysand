// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use wasm_bindgen::prelude::*;

#[cfg(feature = "browser")]
use sysand_core::commands::new::do_new;

pub mod env;
pub mod io;

#[cfg(feature = "browser")]
mod local_storage_utils;

#[wasm_bindgen(js_name = init_logger)]
pub fn init_logger() {
    let _ = console_log::init_with_level(log::Level::Debug);
}

#[wasm_bindgen(js_name = ensure_debug_hook)]
pub fn ensure_debug_hook() {
    console_error_panic_hook::set_once();
}

#[cfg(feature = "browser")]
#[wasm_bindgen(js_name = clear_local_storage)]
pub fn clear_local_storage(prefix: &str) -> Result<(), JsValue> {
    let local_storage = local_storage_utils::get_local_browser_storage(prefix).unwrap();
    local_storage.local_storage.clear()
}

#[cfg(feature = "browser")]
#[wasm_bindgen(js_name = do_new_js_local_storage)]
pub fn do_new_js_local_storage(
    name: String,
    version: String,
    prefix: &str,
    root_path: &str,
    license: Option<String>,
) -> Result<(), JsValue> {
    use typed_path::Utf8UnixPath;

    do_new(
        name,
        version,
        license,
        &mut io::local_storage::ProjectLocalBrowserStorage {
            vfs: local_storage_utils::get_local_browser_storage(prefix).unwrap(),
            root_path: Utf8UnixPath::new(root_path).to_path_buf(),
        },
    )
    .map_err(|e| JsValue::from_str(&e.to_string()))
}

#[cfg(feature = "browser")]
#[wasm_bindgen(js_name = do_env_js_local_storage)]
pub fn do_env_js_local_storage(prefix: &str, root_path: &str) -> Result<(), JsValue> {
    use typed_path::Utf8UnixPath;

    use crate::env::local_storage::{DEFAULT_ENV_NAME, empty_environment_local_storage};

    empty_environment_local_storage(prefix, Utf8UnixPath::new(root_path).join(DEFAULT_ENV_NAME))
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

    Ok(())
}
