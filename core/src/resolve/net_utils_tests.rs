// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use super::{create_reqwest_client, install_default_crypto_provider};

/// The reason [`install_default_crypto_provider`] exists. `reqwest` is taken
/// with `rustls-no-provider`, so it has no compiled-in provider to fall back
/// on and `build()` fails outright unless something installed one -- and
/// `create_reqwest_client` is used by callers that know nothing about rustls.
#[test]
fn create_reqwest_client_needs_no_provider_from_the_caller() {
    assert!(
        create_reqwest_client().is_ok(),
        "building the client must not require the caller to install a \
         crypto provider first"
    );
}

/// Installing must stay idempotent. `install_default` returns `Err` once a
/// provider is set, and discarding that error is what lets an application
/// install its own provider first and keep it -- so a second call must be
/// harmless rather than a panic or an override.
#[test]
fn install_default_crypto_provider_is_idempotent() {
    install_default_crypto_provider();
    install_default_crypto_provider();

    assert!(
        rustls::crypto::CryptoProvider::get_default().is_some(),
        "a provider must be installed after the call"
    );
}
