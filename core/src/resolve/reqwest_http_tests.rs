// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

#![allow(unused_imports)]

use std::sync::Arc;

use fluent_uri::Iri;

use crate::{
    auth::Unauthenticated,
    project::ProjectRead as _,
    resolve::{
        ResolutionInfo, ResolutionOutcome, ResolveRead, ResolveReadAsync as _,
        net_utils::create_reqwest_client,
    },
};

fn resolve<R: ResolveRead>(
    resolver: &R,
    iri: &str,
) -> Result<ResolutionOutcome<R::ResolvedStorages>, R::Error> {
    let resolve = ResolutionInfo::iri(Iri::parse(iri).unwrap().into());
    resolver.resolve_read(&resolve)
}

#[test]
fn basic_http_src_url_non_lax() -> Result<(), Box<dyn std::error::Error>> {
    let mut server = mockito::Server::new();

    let host = server.host_with_port();

    let info_mock = server
        .mock("GET", "/foo/.project.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"basic_http_src_url","version":"1.2.3"}"#)
        .expect(1)
        .create();

    let meta_mock = server
        .mock("GET", "/foo/.meta.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect(1)
        .create();

    let client = create_reqwest_client()?;

    let resolver = super::HTTPResolverAsync {
        client,
        lax: false,
        auth_policy: Arc::new(Unauthenticated {}), //prefer_ranged: true,
    }
    .to_tokio_sync(Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap(),
    ));

    let ResolutionOutcome::Resolved(projects) = resolve(&resolver, &format!("http://{host}/foo/"))?
    else {
        panic!()
    };

    for project in projects {
        let project = project?;

        let (Some(info), Some(meta)) = project
            .get_project()?
            .and_then(crate::project::ProjectDefinition::into_json)
            .unzip()
        else {
            panic!()
        };

        assert_eq!(info.name, "basic_http_src_url");
        assert_eq!(meta.created, "0000-00-00T00:00:00.123456789Z");
    }

    info_mock.assert();
    meta_mock.assert();

    Ok(())
}

fn template_basic_http_url_lax(with_slash: bool) -> Result<(), Box<dyn std::error::Error>> {
    let client = create_reqwest_client()?;

    let resolver = super::HTTPResolverAsync {
        client,
        lax: true,
        auth_policy: Arc::new(Unauthenticated {}),
    }
    .to_tokio_sync(Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap(),
    ));

    let url = if with_slash {
        "http://www.example.invalid/foo/"
    } else {
        "http://www.example.invalid/foo"
    };

    let ResolutionOutcome::Resolved(projects) = resolve(&resolver, url)? else {
        panic!()
    };
    let projects: Vec<super::HTTPProjectAsync<Unauthenticated>> = projects
        .into_iter()
        .map(|x| x.unwrap().into_inner())
        .collect();

    assert_eq!(projects.len(), 2);

    let mut found_src = false;
    let mut found_kpar = false;

    for project in projects {
        match project {
            crate::resolve::reqwest_http::HTTPProjectAsync::HTTPSrcProject(_) => {
                found_src = true;
            }
            crate::resolve::reqwest_http::HTTPProjectAsync::HTTPKParProjectDownloaded(_) => {
                found_kpar = true;
            }
        }
    }

    assert!(found_kpar);
    assert!(found_src);

    Ok(())
}

#[test]
fn basic_http_url_lax_with_slash_not_prefer_ranged() -> Result<(), Box<dyn std::error::Error>> {
    template_basic_http_url_lax(true)
}

#[test]
fn basic_http_url_lax_without_slash_not_prefer_ranged() -> Result<(), Box<dyn std::error::Error>> {
    template_basic_http_url_lax(false)
}
