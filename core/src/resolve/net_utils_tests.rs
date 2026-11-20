// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::assert_matches;
use std::{
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use super::{
    HttpOptions, create_reqwest_client, create_reqwest_client_with,
    install_default_crypto_provider, jittered, parse_retry_after, retry_after, retry_reason,
};

/// `reqwest` has no compiled-in provider, so `build()` fails unless
/// [`install_default_crypto_provider`] ran.
#[test]
fn create_reqwest_client_needs_no_provider_from_the_caller() {
    assert!(
        create_reqwest_client().is_ok(),
        "building the client must not require the caller to install a \
         crypto provider first"
    );
}

/// A second call is harmless, and an application's own provider installed
/// first is kept.
#[test]
fn install_default_crypto_provider_is_idempotent() {
    install_default_crypto_provider();
    install_default_crypto_provider();

    assert!(
        rustls::crypto::CryptoProvider::get_default().is_some(),
        "a provider must be installed after the call"
    );
}

fn options(read_timeout: Duration, retries: u32) -> HttpOptions {
    HttpOptions {
        read_timeout,
        retries,
        ..HttpOptions::default()
    }
}

fn send(
    client: &reqwest_middleware::ClientWithMiddleware,
    method: reqwest::Method,
    url: &str,
) -> reqwest_middleware::Result<reqwest::Response> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(client.request(method, url).send())
}

/// A server that accepts connections and never answers trips the read
/// timeout, and each retry opens a connection of its own.
#[test]
fn stalled_server_times_out_after_retries() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = accepted.clone();
    std::thread::spawn(move || {
        // Kept open, so the client waits for a response
        let _held: Vec<_> = listener
            .incoming()
            .inspect(|_| _ = counter.fetch_add(1, Ordering::SeqCst))
            .collect();
    });

    let client = create_reqwest_client_with(options(Duration::from_millis(200), 2)).unwrap();
    let started = Instant::now();
    let err = send(&client, reqwest::Method::GET, &format!("http://{addr}/")).unwrap_err();

    assert_matches!(&err, reqwest_middleware::Error::Reqwest(e) if e.is_timeout(), "expected a timeout, got {err:?}");
    assert_eq!(accepted.load(Ordering::SeqCst), 3);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn server_error_is_retried_and_last_response_returned() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("GET", "/index.json")
        .with_status(503)
        .expect(3)
        .create();

    let client = create_reqwest_client_with(options(Duration::from_secs(5), 2)).unwrap();
    let response = send(
        &client,
        reqwest::Method::GET,
        &format!("{}/index.json", server.url()),
    )
    .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    mock.assert();
}

#[test]
fn client_error_is_not_retried() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("GET", "/index.json")
        .with_status(404)
        .expect(1)
        .create();

    let client = create_reqwest_client_with(options(Duration::from_secs(5), 2)).unwrap();
    send(
        &client,
        reqwest::Method::GET,
        &format!("{}/index.json", server.url()),
    )
    .unwrap();

    mock.assert();
}

/// A failed TLS handshake fails the same way every time, so it is not
/// retried.
#[test]
fn tls_failure_is_not_retried() {
    // A plain HTTP server answers the TLS handshake with something that is
    // not TLS
    let server = mockito::Server::new();
    let url = server.url().replacen("http://", "https://", 1);

    let client = create_reqwest_client_with(options(Duration::from_secs(5), 0)).unwrap();
    let result = send(&client, reqwest::Method::GET, &url);

    assert_matches!(&result, Err(reqwest_middleware::Error::Reqwest(e)) if e.is_connect(), "expected a connection error, got {result:?}");
    assert_eq!(retry_reason(&result), None);
}

/// The server may have acted on a `POST` that failed, so it is sent once.
#[test]
fn non_idempotent_request_is_not_retried() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("POST", "/upload")
        .with_status(503)
        .expect(1)
        .create();

    let client = create_reqwest_client_with(options(Duration::from_secs(5), 2)).unwrap();
    send(
        &client,
        reqwest::Method::POST,
        &format!("{}/upload", server.url()),
    )
    .unwrap();

    mock.assert();
}

#[test]
fn retry_after_is_seconds_or_a_future_date() {
    let now: jiff::Timestamp = "2025-01-01T00:00:00Z".parse().unwrap();

    assert_eq!(parse_retry_after("7", now), Some(Duration::from_secs(7)));
    assert_eq!(parse_retry_after("0", now), Some(Duration::ZERO));
    assert_eq!(
        parse_retry_after("Wed, 01 Jan 2025 00:00:10 GMT", now),
        Some(Duration::from_secs(10))
    );
    assert_eq!(
        parse_retry_after("Fri, 01 Jan 2000 00:00:00 GMT", now),
        None
    );
    assert_eq!(parse_retry_after("-1", now), None);
    assert_eq!(parse_retry_after("soon", now), None);
}

#[test]
fn retry_after_is_read_only_from_429_and_503() {
    let response = |status: u16| {
        reqwest::Response::from(
            http::Response::builder()
                .status(status)
                .header("retry-after", "3")
                .body("")
                .unwrap(),
        )
    };

    assert_eq!(retry_after(&response(429)), Some(Duration::from_secs(3)));
    assert_eq!(retry_after(&response(503)), Some(Duration::from_secs(3)));
    assert_eq!(retry_after(&response(500)), None);
}

#[test]
fn jitter_adds_up_to_the_delay_again() {
    let delay = Duration::from_secs(1);
    let waits: Vec<_> = std::iter::repeat_with(|| jittered(delay))
        .take(20)
        .collect();

    assert!(
        waits.iter().all(|w| (delay..=2 * delay).contains(w)),
        "{waits:?}"
    );
    assert!(waits.iter().any(|w| *w != waits[0]), "{waits:?}");
}
