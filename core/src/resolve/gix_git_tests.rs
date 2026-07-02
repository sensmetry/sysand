// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use fluent_uri::Iri;

use crate::resolve::{ResolutionInfo, ResolutionOutcome, ResolveRead, gix_git::GitResolver};

fn un_once<T>(x: &mut std::iter::Once<T>) -> T {
    x.next().unwrap()
}

fn resolve<R: ResolveRead>(
    resolver: &R,
    iri: &str,
) -> Result<ResolutionOutcome<R::ResolvedStorages>, R::Error> {
    let resolve = ResolutionInfo::iri(Iri::parse(iri).unwrap().into());
    resolver.resolve_read(&resolve)
}

#[test]
fn basic_url_examples() -> Result<(), Box<dyn std::error::Error>> {
    let res = GitResolver {};

    let ResolutionOutcome::Resolved(mut one_http_proj) =
        resolve(&res, "http://www.example.com/proj")?
    else {
        panic!("expected http url to resolve");
    };
    assert_eq!(
        un_once(&mut one_http_proj).unwrap().url.to_string(),
        "http://www.example.com/proj"
    );

    let ResolutionOutcome::Resolved(mut one_https_proj) =
        resolve(&res, "https://www.example.com/proj")?
    else {
        panic!("expected https url to resolve");
    };
    assert_eq!(
        un_once(&mut one_https_proj).unwrap().url.to_string(),
        "https://www.example.com/proj"
    );

    let ResolutionOutcome::Resolved(mut one_ssh_proj) =
        resolve(&res, "ssh://www.example.com/proj")?
    else {
        panic!("expected ssh url to resolve");
    };
    assert_eq!(
        un_once(&mut one_ssh_proj).unwrap().url.to_string(),
        "ssh://www.example.com/proj"
    );

    let ResolutionOutcome::Resolved(mut one_file_proj) =
        resolve(&res, "file://www.example.com/proj")?
    else {
        panic!("expected file url to resolve");
    };
    assert_eq!(
        un_once(&mut one_file_proj).unwrap().url.to_string(),
        "file://www.example.com/proj"
    );

    let ResolutionOutcome::Resolved(mut one_git_http_proj) =
        resolve(&res, "git+http://www.example.com/proj")?
    else {
        panic!("expected git+http url to resolve");
    };
    assert_eq!(
        un_once(&mut one_git_http_proj).unwrap().url.to_string(),
        "http://www.example.com/proj"
    );

    let ResolutionOutcome::Resolved(mut one_git_https_proj) =
        resolve(&res, "git+https://www.example.com/proj")?
    else {
        panic!("expected git+https url to resolve");
    };
    assert_eq!(
        un_once(&mut one_git_https_proj).unwrap().url.to_string(),
        "https://www.example.com/proj"
    );

    let ResolutionOutcome::Resolved(mut one_git_ssh_proj) =
        resolve(&res, "git+ssh://www.example.com/proj")?
    else {
        panic!("expected git+ssh url to resolve");
    };
    assert_eq!(
        un_once(&mut one_git_ssh_proj).unwrap().url.to_string(),
        "ssh://www.example.com/proj"
    );

    let ResolutionOutcome::Resolved(mut one_git_file_proj) =
        resolve(&res, "git+file://www.example.com/proj")?
    else {
        panic!("expected git+file url to resolve");
    };
    assert_eq!(
        un_once(&mut one_git_file_proj).unwrap().url.to_string(),
        "file://www.example.com/proj"
    );

    Ok(())
}
