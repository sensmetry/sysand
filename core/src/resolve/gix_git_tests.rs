use crate::resolve::{ResolutionOutcome, ResolveRead, gix_git::GitResolver};

fn un_once<T>(x: &mut std::iter::Once<T>) -> T {
    x.next().unwrap()
}

#[test]
fn basic_url_examples() -> Result<(), Box<dyn std::error::Error>> {
    let resolver = GitResolver {};

    let ResolutionOutcome::Resolved(mut one_http_proj) =
        resolver.resolve_read_raw("http://www.example.com/proj")?
    else {
        panic!("expected http url to resolve");
    };
    assert_eq!(
        un_once(&mut one_http_proj).unwrap().url.to_string(),
        "http://www.example.com/proj"
    );

    let ResolutionOutcome::Resolved(mut one_https_proj) =
        resolver.resolve_read_raw("https://www.example.com/proj")?
    else {
        panic!("expected https url to resolve");
    };
    assert_eq!(
        un_once(&mut one_https_proj).unwrap().url.to_string(),
        "https://www.example.com/proj"
    );

    let ResolutionOutcome::Resolved(mut one_ssh_proj) =
        resolver.resolve_read_raw("ssh://www.example.com/proj")?
    else {
        panic!("expected ssh url to resolve");
    };
    assert_eq!(
        un_once(&mut one_ssh_proj).unwrap().url.to_string(),
        "ssh://www.example.com/proj"
    );

    let ResolutionOutcome::Resolved(mut one_file_proj) =
        resolver.resolve_read_raw("file://www.example.com/proj")?
    else {
        panic!("expected file url to resolve");
    };
    assert_eq!(
        un_once(&mut one_file_proj).unwrap().url.to_string(),
        "file://www.example.com/proj"
    );

    let ResolutionOutcome::Resolved(mut one_git_http_proj) =
        resolver.resolve_read_raw("git+http://www.example.com/proj")?
    else {
        panic!("expected git+http url to resolve");
    };
    assert_eq!(
        un_once(&mut one_git_http_proj).unwrap().url.to_string(),
        "http://www.example.com/proj"
    );

    let ResolutionOutcome::Resolved(mut one_git_https_proj) =
        resolver.resolve_read_raw("git+https://www.example.com/proj")?
    else {
        panic!("expected git+https url to resolve");
    };
    assert_eq!(
        un_once(&mut one_git_https_proj).unwrap().url.to_string(),
        "https://www.example.com/proj"
    );

    let ResolutionOutcome::Resolved(mut one_git_ssh_proj) =
        resolver.resolve_read_raw("git+ssh://www.example.com/proj")?
    else {
        panic!("expected git+ssh url to resolve");
    };
    assert_eq!(
        un_once(&mut one_git_ssh_proj).unwrap().url.to_string(),
        "ssh://www.example.com/proj"
    );

    let ResolutionOutcome::Resolved(mut one_git_file_proj) =
        resolver.resolve_read_raw("git+file://www.example.com/proj")?
    else {
        panic!("expected git+file url to resolve");
    };
    assert_eq!(
        un_once(&mut one_git_file_proj).unwrap().url.to_string(),
        "file://www.example.com/proj"
    );

    Ok(())
}
