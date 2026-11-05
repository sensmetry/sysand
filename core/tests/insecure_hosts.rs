// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

// These tests need a rustls crypto provider, which only a `tls-*` feature
// installs; `--features networking` deliberately has none.
#![cfg(all(
    feature = "networking",
    any(feature = "tls-aws-lc-rs", feature = "tls-ring")
))]

use std::{
    io::{Read as _, Write as _},
    net::TcpListener,
    sync::Arc,
    time::Duration,
};

use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject as _};
use sysand_core::resolve::net_utils::{
    HttpOptions, InsecureHost, create_reqwest_client_with, install_default_crypto_provider,
};

fn get(insecure: &[&str], url: &str) -> reqwest_middleware::Result<reqwest::Response> {
    let client = create_reqwest_client_with(&HttpOptions {
        retries: 0,
        read_timeout: Duration::from_secs(5),
        insecure_hosts: insecure.iter().map(|h| h.parse().unwrap()).collect(),
        ..HttpOptions::default()
    })
    .unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(client.get(url).send())
}

/// Serve `response` over TLS, on a free port of `127.0.0.1`, with the
/// self-signed certificate in `tests/data`. The certificate names no host, but
/// no test depends on that: it is rejected anyway for being self-signed, and
/// the client that skips verification accepts it whatever it says.
fn serve(response: String) -> u16 {
    install_default_crypto_provider();
    let config = Arc::new(
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from_pem_slice(include_bytes!("data/cert.pem")).unwrap()],
                PrivateKeyDer::from_pem_slice(include_bytes!("data/key.pem")).unwrap(),
            )
            .unwrap(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let (config, response) = (config.clone(), response.clone());
            std::thread::spawn(move || {
                let conn = rustls::ServerConnection::new(config).unwrap();
                let mut tls = rustls::StreamOwned::new(conn, stream.unwrap());
                // Reading the request, which is small, completes the
                // handshake. A failed handshake, which is what most of these
                // tests expect, makes it an error.
                if tls.read(&mut [0; 1024]).is_err() {
                    return;
                }
                _ = tls.write_all(response.as_bytes());
                tls.conn.send_close_notify();
                _ = tls.flush();
            });
        }
    });
    port
}

fn ok() -> String {
    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_owned()
}

fn redirect(to: &str) -> String {
    format!(
        "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

#[test]
fn self_signed_certificate_is_rejected_by_default() {
    let port = serve(ok());
    get(&[], &format!("https://127.0.0.1:{port}/")).unwrap_err();
}

#[test]
fn listed_host_is_accepted() {
    let port = serve(ok());
    let response = get(&["127.0.0.1"], &format!("https://127.0.0.1:{port}/")).unwrap();
    assert_eq!(response.status(), 200);
}

#[test]
fn other_host_is_still_verified() {
    let port = serve(ok());
    get(&["localhost"], &format!("https://127.0.0.1:{port}/")).unwrap_err();
}

#[test]
fn redirect_between_listed_hosts_is_followed_without_verification() {
    let target = serve(ok());
    let origin = serve(redirect(&format!("https://localhost:{target}/")));
    let url = format!("https://127.0.0.1:{origin}/");
    assert_eq!(
        get(&["127.0.0.1", "localhost"], &url).unwrap().status(),
        200
    );
}

#[test]
fn redirect_from_listed_host_to_unlisted_host_is_verified() {
    let target = serve(ok());
    let origin = serve(redirect(&format!("https://localhost:{target}/")));
    // Followed, and failed on the target's certificate, which would have been
    // accepted had the target been listed
    get(&["127.0.0.1"], &format!("https://127.0.0.1:{origin}/")).unwrap_err();
}

#[test]
fn redirect_from_listed_host_to_unlisted_host_is_followed() {
    let mut target = mockito::Server::new();
    let target_mock = target.mock("GET", "/target").with_body("ok").create();
    let origin = serve(redirect(&url_of(&target, "localhost", "/target")));

    let response = get(&["127.0.0.1"], &format!("https://127.0.0.1:{origin}/")).unwrap();

    assert_eq!(response.status(), 200);
    target_mock.assert();
}

/// The origin is plain `http`, as a TLS one with a self-signed certificate
/// would fail verification before it could redirect anywhere.
#[test]
fn redirect_from_unlisted_host_to_listed_host_is_followed_without_verification() {
    let target = serve(ok());
    let mut server = mockito::Server::new();
    let origin = server
        .mock("GET", "/origin")
        .with_status(302)
        .with_header("location", &format!("https://localhost:{target}/"))
        .expect(2)
        .create();
    let url = format!("http://127.0.0.1:{}/origin", server.socket_address().port());

    // Unlisted, the target's certificate is rejected
    get(&[], &url).unwrap_err();
    assert_eq!(get(&["localhost"], &url).unwrap().status(), 200);
    // Once for each request
    origin.assert();
}

/// Only `localhost` is listed, so the hops alternate between the client that
/// verifies and the one that does not.
#[test]
fn redirect_chain_through_listed_and_unlisted_hosts_is_followed() {
    let mut server = mockito::Server::new();
    let end = server.mock("GET", "/end").with_body("ok").create();
    let tls = serve(redirect(&url_of(&server, "127.0.0.1", "/end")));
    server
        .mock("GET", "/b")
        .with_status(302)
        .with_header("location", &format!("https://localhost:{tls}/"))
        .create();
    server
        .mock("GET", "/a")
        .with_status(302)
        .with_header("location", &url_of(&server, "127.0.0.1", "/b"))
        .create();

    let response = get(&["localhost"], &url_of(&server, "localhost", "/a")).unwrap();

    assert_eq!(response.status(), 200);
    end.assert();
}

#[test]
fn redirect_loop_between_listed_and_unlisted_hosts_ends_in_an_error() {
    let mut server = mockito::Server::new();
    server
        .mock("GET", "/a")
        .with_status(302)
        .with_header("location", &url_of(&server, "localhost", "/b"))
        .expect_at_least(2)
        .create();
    server
        .mock("GET", "/b")
        .with_status(302)
        .with_header("location", &url_of(&server, "127.0.0.1", "/a"))
        .expect_at_least(2)
        .create();

    get(&["localhost"], &url_of(&server, "127.0.0.1", "/a")).unwrap_err();
}

/// A URL on a mock server, which speaks plain `http`, so tests using it are
/// about which redirects are followed, not about certificate verification.
/// The server is reachable as both `127.0.0.1` and `localhost`, which are
/// different hosts as far as the list is concerned.
fn url_of(server: &mockito::Server, host: &str, path: &str) -> String {
    format!("http://{host}:{}{path}", server.socket_address().port())
}

#[test]
fn redirect_between_listed_hosts_over_http_is_followed() {
    let mut server = mockito::Server::new();
    let target = url_of(&server, "localhost", "/target");
    server
        .mock("GET", "/origin")
        .with_status(302)
        .with_header("location", &target)
        .create();
    let target_mock = server.mock("GET", "/target").with_body("ok").create();

    let response = get(
        &["127.0.0.1", "localhost"],
        &url_of(&server, "127.0.0.1", "/origin"),
    )
    .unwrap();

    assert_eq!(response.status(), 200);
    target_mock.assert();
}

#[test]
fn redirect_loop_between_listed_hosts_ends_in_an_error() {
    let mut server = mockito::Server::new();
    let origin = url_of(&server, "127.0.0.1", "/origin");
    server
        .mock("GET", "/origin")
        .with_status(302)
        .with_header("location", &origin)
        .expect_at_least(2)
        .create();

    get(&["127.0.0.1"], &origin).unwrap_err();
}

#[test]
fn credentials_are_not_sent_to_another_origin() {
    let mut server = mockito::Server::new();
    let target = url_of(&server, "localhost", "/target");
    server
        .mock("GET", "/origin")
        .match_header("authorization", "secret")
        .with_status(307)
        .with_header("location", &target)
        .create();
    let target_mock = server
        .mock("GET", "/target")
        .match_header("authorization", mockito::Matcher::Missing)
        .with_body("ok")
        .create();

    let client = create_reqwest_client_with(&HttpOptions {
        insecure_hosts: vec!["127.0.0.1".parse().unwrap()],
        ..HttpOptions::default()
    })
    .unwrap();
    let response = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(
            client
                .get(url_of(&server, "127.0.0.1", "/origin"))
                .header("authorization", "secret")
                .send(),
        )
        .unwrap();

    assert_eq!(response.status(), 200);
    target_mock.assert();
}

#[test]
fn see_other_turns_post_into_get() {
    let mut server = mockito::Server::new();
    let target = url_of(&server, "127.0.0.1", "/target");
    server
        .mock("POST", "/origin")
        .with_status(303)
        .with_header("location", &target)
        .create();
    let target_mock = server.mock("GET", "/target").with_body("ok").create();

    let client = create_reqwest_client_with(&HttpOptions {
        insecure_hosts: vec!["127.0.0.1".parse().unwrap()],
        ..HttpOptions::default()
    })
    .unwrap();
    let response = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(
            client
                .post(url_of(&server, "127.0.0.1", "/origin"))
                .body("data")
                .send(),
        )
        .unwrap();

    assert_eq!(response.status(), 200);
    target_mock.assert();
}

#[test]
fn hosts_are_normalised() {
    assert_eq!(
        "LocalHost".parse::<InsecureHost>().unwrap(),
        InsecureHost::Domain("localhost".into())
    );
    assert_eq!(
        "[::1]".parse::<InsecureHost>().unwrap(),
        InsecureHost::Ip(std::net::Ipv6Addr::LOCALHOST.into())
    );
    assert_eq!("[::1]".parse::<InsecureHost>().unwrap().to_string(), "::1");
    for bad in ["", "::1", "a:1", "https://a", "a/b", "a b"] {
        bad.parse::<InsecureHost>().unwrap_err();
    }
}
