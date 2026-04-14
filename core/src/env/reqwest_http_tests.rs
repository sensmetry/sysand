use std::sync::Arc;

use crate::{
    auth::Unauthenticated,
    env::{ReadEnvironment, ReadEnvironmentAsync},
    resolve::{net_utils::create_reqwest_client, reqwest_http::HTTPProjectAsync},
};

#[test]
fn test_uri_examples() -> Result<(), Box<dyn std::error::Error>> {
    let env = super::HTTPEnvironmentAsync {
        client: create_reqwest_client()?,
        base_url: url::Url::parse("https://www.example.com/a/b")?,
        prefer_src: true,
        auth_policy: Arc::new(Unauthenticated {}),
        // try_ranged: false,
    };

    assert_eq!(env.root_url().to_string(), "https://www.example.com/a/b");
    assert_eq!(
        env.entries_url()?.to_string(),
        "https://www.example.com/a/entries.txt"
    );
    assert_eq!(
        env.versions_url("urn:kpar:b")?.to_string(),
        "https://www.example.com/a/621a5fdf587a3ecc878a98c8be2240dd5bbe561860d11f4da1ece4a4fe2fb8b5/versions.txt"
    );
    assert_eq!(
        env.project_kpar_url("urn:kpar:b", "1.0.0")?.to_string(),
        "https://www.example.com/a/621a5fdf587a3ecc878a98c8be2240dd5bbe561860d11f4da1ece4a4fe2fb8b5/1.0.0.kpar"
    );
    assert_eq!(
        env.project_src_url("urn:kpar:b", "1.0.0")?.to_string(),
        "https://www.example.com/a/621a5fdf587a3ecc878a98c8be2240dd5bbe561860d11f4da1ece4a4fe2fb8b5/1.0.0.kpar/"
    );

    Ok(())
}

#[test]
fn test_basic_enumerations() -> Result<(), Box<dyn std::error::Error>> {
    let mut server = mockito::Server::new();

    let host = server.url();

    let env = super::HTTPEnvironmentAsync {
        client: create_reqwest_client()?,
        base_url: url::Url::parse(&host)?,
        prefer_src: true,
        auth_policy: Arc::new(Unauthenticated {}),
        //try_ranged: false,
    }
    .to_tokio_sync(Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?,
    ));

    let entries_mock = server
        .mock("GET", "/entries.txt")
        .with_status(200)
        .with_header("content-type", "text/plain")
        .with_body("urn:kpar:a\nurn:kpar:b")
        .create();

    let versions_a_mock = server
        .mock(
            "GET",
            "/cf642a3abc90961d460b5159fb9dd82c011a659bb59d1df976233675b23d78b0/versions.txt",
        )
        .with_status(200)
        .with_header("content-type", "text/plain")
        .with_body("1.0.0")
        .create();

    let versions_b_mock = server
        .mock(
            "GET",
            "/621a5fdf587a3ecc878a98c8be2240dd5bbe561860d11f4da1ece4a4fe2fb8b5/versions.txt",
        )
        .with_status(200)
        .with_header("content-type", "text/plain")
        .with_body("1.0.0\n2.0.0")
        .create();

    let uris: Result<Vec<_>, _> = env.uris()?.collect();
    let uris = uris?;

    assert_eq!(uris.len(), 2);
    assert!(uris.contains(&"urn:kpar:a".to_string()));
    assert!(uris.contains(&"urn:kpar:b".to_string()));

    let a_versions: Result<Vec<_>, _> = env.versions("urn:kpar:a")?.collect();
    let a_versions = a_versions?;

    assert_eq!(a_versions.len(), 1);
    assert!(a_versions.contains(&"1.0.0".to_string()));

    let b_versions: Result<Vec<_>, _> = env.versions("urn:kpar:b")?.collect();
    let b_versions = b_versions?;

    assert_eq!(b_versions.len(), 2);
    assert!(b_versions.contains(&"1.0.0".to_string()));
    assert!(b_versions.contains(&"2.0.0".to_string()));

    entries_mock.assert();
    versions_a_mock.assert();
    versions_b_mock.assert();

    Ok(())
}

#[test]
fn test_kpar_fallback() -> Result<(), Box<dyn std::error::Error>> {
    let mut server = mockito::Server::new();

    let host = server.url();

    let env = super::HTTPEnvironmentAsync {
        client: create_reqwest_client()?,
        base_url: url::Url::parse(&host)?,
        prefer_src: true,
        auth_policy: Arc::new(Unauthenticated {}),
        //try_ranged: false,
    }
    .to_tokio_sync(Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?,
    ));

    let kpar_mock = server
        .mock(
            "HEAD", // urn:kpar:a
            "/cf642a3abc90961d460b5159fb9dd82c011a659bb59d1df976233675b23d78b0/1.0.0.kpar",
        )
        .with_status(200)
        .with_header("content-type", "application/zip")
        .with_body("")
        .create();

    let project = env.get_project("urn:kpar:a", "1.0.0")?;

    let HTTPProjectAsync::HTTPKParProjectDownloaded(_) = project.inner else {
        panic!("Expected to resolve to KPar project");
    };

    kpar_mock.assert();

    Ok(())
}

#[test]
fn test_src_fallback() -> Result<(), Box<dyn std::error::Error>> {
    let mut server = mockito::Server::new();

    let host = server.url();

    let env = super::HTTPEnvironmentAsync {
        client: create_reqwest_client()?,
        base_url: url::Url::parse(&host)?,
        prefer_src: false,
        auth_policy: Arc::new(Unauthenticated {}),
        //try_ranged: false,
    }
    .to_tokio_sync(Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?,
    ));

    let src_mock = server
            .mock(
                "HEAD", // urn:kpar:a
                "/cf642a3abc90961d460b5159fb9dd82c011a659bb59d1df976233675b23d78b0/1.0.0.kpar/.project.json",
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("")
            .create();

    let project = env.get_project("urn:kpar:a", "1.0.0")?;

    let HTTPProjectAsync::HTTPSrcProject(_) = project.inner else {
        panic!("Expected to resolve to src project");
    };

    src_mock.assert();

    Ok(())
}

#[test]
fn test_kpar_preference() -> Result<(), Box<dyn std::error::Error>> {
    let mut server = mockito::Server::new();

    let host = server.url();

    let env = super::HTTPEnvironmentAsync {
        client: create_reqwest_client()?,
        base_url: url::Url::parse(&host)?,
        prefer_src: false,
        auth_policy: Arc::new(Unauthenticated {}),
        //try_ranged: false,
    }
    .to_tokio_sync(Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?,
    ));

    let kpar_mock = server
        .mock(
            "HEAD", // urn:kpar:a
            "/cf642a3abc90961d460b5159fb9dd82c011a659bb59d1df976233675b23d78b0/1.0.0.kpar",
        )
        .with_status(200)
        .with_header("content-type", "application/zip")
        .with_body("")
        .create();

    let src_mock = server
            .mock(
                "HEAD", // urn:kpar:a
                "/cf642a3abc90961d460b5159fb9dd82c011a659bb59d1df976233675b23d78b0/1.0.0.kpar/.project.json",
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("")
            .expect_at_most(0)
            .create();

    let project = env.get_project("urn:kpar:a", "1.0.0")?;

    let HTTPProjectAsync::HTTPKParProjectDownloaded(_) = project.inner else {
        panic!("Expected to resolve to KPar project");
    };

    src_mock.assert();
    kpar_mock.assert();

    Ok(())
}

#[test]
fn test_src_preference() -> Result<(), Box<dyn std::error::Error>> {
    let mut server = mockito::Server::new();

    let host = server.url();

    let env = super::HTTPEnvironmentAsync {
        client: create_reqwest_client()?,
        base_url: url::Url::parse(&host)?,
        prefer_src: true,
        auth_policy: Arc::new(Unauthenticated {}),
        //try_ranged: false,
    }
    .to_tokio_sync(Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?,
    ));

    let kpar_mock = server
        .mock(
            "HEAD", // urn:kpar:a
            "/cf642a3abc90961d460b5159fb9dd82c011a659bb59d1df976233675b23d78b0/1.0.0.kpar",
        )
        .with_status(200)
        .with_header("content-type", "application/zip")
        .with_body("")
        .expect_at_most(0)
        .create();

    let src_mock = server
            .mock(
                "HEAD", // urn:kpar:a
                "/cf642a3abc90961d460b5159fb9dd82c011a659bb59d1df976233675b23d78b0/1.0.0.kpar/.project.json",
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("")
            .create();

    let project = env.get_project("urn:kpar:a", "1.0.0")?;

    let HTTPProjectAsync::HTTPSrcProject(_) = project.inner else {
        panic!("Expected to resolve to src project");
    };

    src_mock.assert();
    kpar_mock.assert();

    Ok(())
}
