#![allow(unused_imports)]

use std::sync::Arc;

use crate::{
    auth::Unauthenticated,
    project::ProjectRead,
    resolve::{ResolutionOutcome, ResolveRead, ResolveReadAsync, net_utils::create_reqwest_client},
};

#[test]
fn test_basic_http_src_url_non_lax() -> Result<(), Box<dyn std::error::Error>> {
    let mut server = mockito::Server::new();

    let host = server.host_with_port();

    let info_mock = server
        .mock("GET", "/foo/.project.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"test_basic_http_src_url","version":"1.2.3","usage":[]}"#)
        .create();

    let meta_mock = server
        .mock("GET", "/foo/.meta.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
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

    let ResolutionOutcome::Resolved(projects) =
        resolver.resolve_read_raw(format!("http://{}/foo/", host))?
    else {
        panic!()
    };

    for project in projects {
        let project = project?;

        let (Some(info), Some(meta)) = project.get_project()? else {
            panic!()
        };

        assert_eq!(info.name, "test_basic_http_src_url");
        assert_eq!(meta.created, "0000-00-00T00:00:00.123456789Z");
    }

    info_mock.assert();
    meta_mock.assert();

    Ok(())
}

fn template_basic_http_url_lax(
    with_slash: bool,
    //prefer_ranged: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = create_reqwest_client()?;

    let resolver = super::HTTPResolverAsync {
        client,
        lax: true,
        auth_policy: Arc::new(Unauthenticated {}), //prefer_ranged,
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

    let ResolutionOutcome::Resolved(projects) = resolver.resolve_read_raw(url)? else {
        panic!()
    };
    let projects: Vec<super::HTTPProjectAsync<Unauthenticated>> =
        projects.into_iter().map(|x| x.unwrap().inner).collect();

    assert_eq!(projects.len(), 2);

    let mut found_src = false;
    let mut found_kpar = false;

    for project in projects {
        match project {
            crate::resolve::reqwest_http::HTTPProjectAsync::HTTPSrcProject(_) => {
                found_src = true;
            }
            // crate::resolve::reqwest_http::HTTPProjectAsync::HTTPKParProjectRanged(_) => {
            //     panic!("got a ranged project for invalid url");
            // }
            crate::resolve::reqwest_http::HTTPProjectAsync::HTTPKParProjectDownloaded(_) => {
                found_kpar = true;
            }
        }
    }

    assert!(found_kpar);
    assert!(found_src);

    Ok(())
}

// #[test]
// fn test_basic_http_url_lax_with_slash_prefer_ranged() -> Result<(), Box<dyn std::error::Error>>
// {
//     template_basic_http_url_lax(true, true)
// }

#[test]
fn test_basic_http_url_lax_with_slash_not_prefer_ranged() -> Result<(), Box<dyn std::error::Error>>
{
    template_basic_http_url_lax(true /* false */)
}

// #[test]
// fn test_basic_http_url_lax_without_slash_prefer_ranged()
// -> Result<(), Box<dyn std::error::Error>> {
//     template_basic_http_url_lax(false, true)
// }

#[test]
fn test_basic_http_url_lax_without_slash_not_prefer_ranged()
-> Result<(), Box<dyn std::error::Error>> {
    template_basic_http_url_lax(false /* false */)
}

// NOTE: Testing done in manually, due to lack of range header support in all
//       easy-to-integrate-in-tests HTTP servers.
// #[cfg(feature = "alltests")]
// #[test]
// fn test_resolves_ranged_if_successful() -> Result<(), Box<dyn std::error::Error>> {
//     let cwd = tempfile::tempdir()?;

//     let _buf = {
//         //let mut cursor = std::io::Cursor::new(vec![]);
//         //let mut zip = zip::ZipWriter::new(&mut cursor);

//         let file_path = cwd.path().join("project.kpar");
//         let file = std::fs::File::create(&file_path)?;
//         let mut zip = zip::ZipWriter::new(file);

//         let options = zip::write::SimpleFileOptions::default()
//             .compression_method(zip::CompressionMethod::Stored)
//             .unix_permissions(0o755);

//         zip.start_file("some_root_dir/.project.json", options)?;
//         zip.write_all(
//             br#"{"name":"test_resolves_ranged_if_successful","version":"1.2.3","usage":[]}"#,
//         )?;
//         zip.start_file("some_root_dir/.meta.json", options)?;
//         zip.write_all(br#"{"index":{},"created":"123"}"#)?;
//         zip.start_file("some_root_dir/test.sysml", options)?;
//         zip.write_all(br#"package Test;"#)?;

//         zip.finish().unwrap();

//         //cursor.flush()?;
//         //cursor.into_inner()
//         file_path
//     };

//     let free_port = port_check::free_local_port().unwrap().to_string();

// let mut server = Command::new("uv")
//     .arg("run")
//     .arg("--with")
//     .arg("rangehttpserver")
//     .arg("-m")
//     .arg("RangeHTTPServer")
//     .arg(&free_port)
//     .current_dir(cwd.path())
//     .spawn()?;
// sleep(Duration::from_millis(1000));

//     let client = create_reqwest_client();
//     let resolver = super::HTTPResolverAsync {
//         client,
//         lax: false,
//         prefer_ranged: true,
//     };

//     let ResolutionOutcome::Resolved(projects) =
//         resolver.resolve_read_raw(format!("http://localhost:{}/project.kpar", &free_port))?
//     else {
//         panic!()
//     };

//     let projects: Vec<super::HTTPProject> = projects.into_iter().map(|x| x.unwrap()).collect();
//     assert_eq!(projects.len(), 1);

//     if let crate::resolve::reqwest_http::HTTPProjectAsync::HTTPKParProjectRanged(_) = projects[0] {
//     } else {
//         panic!("expected a ranged project");
//     }

//     server.kill()?;

//     Ok(())
// }

// #[test]
// fn test_resolves_non_ranged_if_unsupported() -> Result<(), Box<dyn std::error::Error>> {
//     let buf = {
//         let mut cursor = std::io::Cursor::new(vec![]);
//         let mut zip = zip::ZipWriter::new(&mut cursor);

//         let options = zip::write::SimpleFileOptions::default()
//             .compression_method(zip::CompressionMethod::Stored)
//             .unix_permissions(0o755);

//         zip.start_file("some_root_dir/.project.json", options)?;
//         zip.write_all(
//             br#"{"name":"test_resolves_non_ranged_if_unsupported","version":"1.2.3","usage":[]}"#,
//         )?;
//         zip.start_file("some_root_dir/.meta.json", options)?;
//         zip.write_all(br#"{"index":{},"created":"123"}"#)?;
//         zip.start_file("some_root_dir/test.sysml", options)?;
//         zip.write_all(br#"package Test;"#)?;

//         zip.finish().unwrap();

//         cursor.flush()?;
//         cursor.into_inner()
//     };

//     let mut server = mockito::Server::new();

//     //let host = server.host_with_port();
//     let url = reqwest::Url::parse(&server.url()).unwrap();

//     // Should only generate a HEAD request
//     let get_kpar = server
//         .mock("HEAD", "/project.kpar")
//         .with_status(200)
//         .with_header("content-type", "application/zip")
//         .with_body(&buf)
//         .create();

//     let client = create_reqwest_client();
//     let resolver = super::HTTPResolverAsync {
//         client,
//         lax: false,
//         prefer_ranged: true,
//     };

//     let ResolutionOutcome::Resolved(projects) =
//         resolver.resolve_read_raw(format!("{}project.kpar", url,))?
//     else {
//         panic!()
//     };

//     let projects: Vec<super::HTTPProject> = projects.into_iter().map(|x| x.unwrap()).collect();
//     assert_eq!(projects.len(), 1);

//     if let crate::resolve::reqwest_http::HTTPProjectAsync::HTTPKParProjectDownloaded(_) = projects[0]
//     {
//     } else {
//         panic!("expected a ranged project");
//     }

//     get_kpar.assert();

//     Ok(())
// }
