// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use super::{
    AllowedMetamodelKind, EndpointKind, PublishError, build_upload_url, check_metamodel,
    check_usage, error_body_to_string, map_publish_response, validate_endpoint_url_shape,
};
use crate::model::InterchangeProjectUsageRaw;
use url::Url;

#[test]
fn build_upload_url_appends_endpoint_path() {
    // `build_upload_url` takes the resolved `api_root`, not the
    // discovery root. Well-known discovery has already chosen
    // `api_root` — this helper just composes `v1/upload` onto it.
    assert_eq!(
        build_upload_url(&Url::parse("https://example.org").unwrap())
            .unwrap()
            .as_str(),
        "https://example.org/v1/upload"
    );
    assert_eq!(
        build_upload_url(&Url::parse("https://example.org/").unwrap())
            .unwrap()
            .as_str(),
        "https://example.org/v1/upload"
    );
    assert_eq!(
        build_upload_url(&Url::parse("https://example.org/api").unwrap())
            .unwrap()
            .as_str(),
        "https://example.org/api/v1/upload"
    );
    assert_eq!(
        build_upload_url(&Url::parse("https://example.org/api/").unwrap())
            .unwrap()
            .as_str(),
        "https://example.org/api/v1/upload"
    );
}

#[test]
fn build_upload_url_preserves_percent_encoded_segments() {
    assert_eq!(
        build_upload_url(&Url::parse("https://example.org/my%20api/").unwrap())
            .unwrap()
            .as_str(),
        "https://example.org/my%20api/v1/upload"
    );
}

#[test]
fn build_upload_url_rejects_upload_endpoint_path() {
    // If the caller hands us an `api_root` that already ends in the
    // upload path, they've pasted the full upload URL in the wrong
    // place. Catch it before we compose `v1/upload/v1/upload`.
    for url in [
        "https://example.org/v1/upload",
        "https://example.org/v1/upload/",
        "https://example.org/api/v1/upload",
        "https://example.org/api/v1/upload/",
    ] {
        let err = build_upload_url(&Url::parse(url).unwrap()).unwrap_err();
        assert!(matches!(err, PublishError::InvalidApiRoot { .. }));
    }
}

#[test]
fn build_upload_url_rejects_query_and_fragment() {
    let err =
        build_upload_url(&Url::parse("https://example.org/index?x=1#frag").unwrap()).unwrap_err();
    assert!(matches!(err, PublishError::InvalidApiRoot { .. }));
}

#[test]
fn build_upload_url_rejects_non_http_scheme() {
    let err = build_upload_url(&Url::parse("ftp://example.org").unwrap()).unwrap_err();
    assert!(matches!(err, PublishError::InvalidApiRoot { .. }));
}

#[test]
fn build_upload_url_rejects_non_hierarchical_url() {
    let err = build_upload_url(&Url::parse("mailto:test@example.org").unwrap()).unwrap_err();
    assert!(matches!(err, PublishError::InvalidApiRoot { .. }));
}

#[test]
fn build_upload_url_rejects_userinfo() {
    for raw in [
        "https://user@example.org/api/",
        "https://user:password@example.org/api/",
    ] {
        let err = build_upload_url(&Url::parse(raw).unwrap()).unwrap_err();
        assert!(matches!(err, PublishError::InvalidApiRoot { .. }));
    }
}

#[test]
fn error_body_to_string_trims_text_content() {
    assert_eq!(error_body_to_string(b"  unauthorized\n"), "unauthorized");
}

#[test]
fn error_body_to_string_extracts_error_from_json() {
    assert_eq!(
        error_body_to_string(br#"{"error":"Invalid token"}"#),
        "Invalid token"
    );
}

#[test]
fn error_body_to_string_reports_empty_body() {
    assert_eq!(error_body_to_string(b" \n\t "), "no error details provided");
}

// --- check_metamodel ---

#[test]
fn check_metamodel_accepts_valid_sysml() {
    assert_eq!(
        check_metamodel("https://www.omg.org/spec/SysML/20250201").unwrap(),
        AllowedMetamodelKind::SysML,
    );
}

#[test]
fn check_metamodel_accepts_valid_kerml() {
    assert_eq!(
        check_metamodel("https://www.omg.org/spec/KerML/20250201").unwrap(),
        AllowedMetamodelKind::KerML,
    );
}

#[test]
fn check_metamodel_rejects_unsupported_metamodel() {
    let err = check_metamodel("https://example.com/some-meta").unwrap_err();
    assert!(matches!(err, PublishError::UnsupportedMetamodel { .. }));
}

#[test]
fn check_metamodel_rejects_invalid_sysml_version() {
    // Valid SysML prefix but non-date version string.
    let err = check_metamodel("https://www.omg.org/spec/SysML/notadate").unwrap_err();
    assert!(matches!(err, PublishError::InvalidMetamodelVersion { .. }));
}

#[test]
fn check_metamodel_rejects_invalid_kerml_version() {
    // Month 13 is not a valid calendar month.
    let err = check_metamodel("https://www.omg.org/spec/KerML/20251301").unwrap_err();
    assert!(matches!(err, PublishError::InvalidMetamodelVersion { .. }));
}

// --- check_usage ---

fn usage(resource: &str) -> InterchangeProjectUsageRaw {
    InterchangeProjectUsageRaw {
        resource: resource.to_string(),
        version_constraint: None,
    }
}

fn usage_with_vc(resource: &str, vc: &str) -> InterchangeProjectUsageRaw {
    InterchangeProjectUsageRaw {
        resource: resource.to_string(),
        version_constraint: Some(vc.to_string()),
    }
}

#[test]
fn check_usage_accepts_valid_sysand_purl() {
    check_usage(&usage("pkg:sysand/acme/widget")).unwrap();
}

#[test]
fn check_usage_accepts_all_known_std_libs() {
    for resource in [
        "https://www.omg.org/spec/KerML/20250201/Data-Type-Library.kpar",
        "https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar",
        "https://www.omg.org/spec/KerML/20250201/Function-Library.kpar",
        "https://www.omg.org/spec/SysML/20250201/Systems-Library.kpar",
        "https://www.omg.org/spec/SysML/20250201/Analysis-Domain-Library.kpar",
        "https://www.omg.org/spec/SysML/20250201/Cause-and-Effect-Domain-Library.kpar",
        "https://www.omg.org/spec/SysML/20250201/Geometry-Domain-Library.kpar",
        "https://www.omg.org/spec/SysML/20250201/Metadata-Domain-Library.kpar",
        "https://www.omg.org/spec/SysML/20250201/Quantities-and-Units-Domain-Library.kpar",
        "https://www.omg.org/spec/SysML/20250201/Requirement-Derivation-Domain-Library.kpar",
    ] {
        check_usage(&usage(resource)).unwrap_or_else(|e| panic!("{resource}: {e}"));
    }
}

#[test]
fn check_usage_rejects_disallowed_usage() {
    // Not a pkg:sysand purl and not a std-lib IRI prefix.
    let err = check_usage(&usage("https://example.com/some/library")).unwrap_err();
    assert!(matches!(err, PublishError::DisallowedUsage { .. }));
}

#[test]
fn check_usage_rejects_invalid_purl() {
    // pkg:sysand prefix present but name segment is syntactically invalid.
    let err = check_usage(&usage("pkg:sysand/publisher/bad__name")).unwrap_err();
    assert!(matches!(err, PublishError::InvalidPurl { .. }));
}

#[test]
fn check_usage_rejects_std_lib_with_version_constraint() {
    let err = check_usage(&usage_with_vc(
        "https://www.omg.org/spec/SysML/20250201/Systems-Library.kpar",
        ">=1.0.0",
    ))
    .unwrap_err();
    assert!(matches!(err, PublishError::StdWithVersionConstraint { .. }));
}

#[test]
fn check_usage_rejects_invalid_std_lib_version() {
    // Valid SysML prefix + known suffix, but invalid date portion.
    let err = check_usage(&usage(
        "https://www.omg.org/spec/SysML/baddate/Systems-Library.kpar",
    ))
    .unwrap_err();
    assert!(matches!(err, PublishError::InvalidStdLibVersion { .. }));
}

#[test]
fn check_usage_rejects_unknown_std_lib() {
    // Valid SysML prefix, no version constraint, but the suffix is not a known library.
    let err = check_usage(&usage(
        "https://www.omg.org/spec/SysML/20250201/Nonexistent-Library.kpar",
    ))
    .unwrap_err();
    assert!(matches!(err, PublishError::UnknownStdLib { .. }));
}

// --- map_publish_response ---

#[test]
fn map_publish_response_400_maps_to_bad_request() {
    let err = map_publish_response(
        400,
        b"bad field",
        "http://example.org/v1/upload",
        "http://example.org/v1/upload",
    )
    .unwrap_err();
    assert!(matches!(err, PublishError::BadRequest(_)));
}

#[test]
fn map_publish_response_200_is_ok_not_new_project() {
    let resp = map_publish_response(
        200,
        b"ok",
        "http://example.org/v1/upload",
        "http://example.org/v1/upload",
    )
    .unwrap();
    assert!(!resp.is_new_project);
    assert_eq!(resp.status, 200);
}

#[test]
fn map_publish_response_201_is_ok_new_project() {
    let resp = map_publish_response(
        201,
        b"created",
        "http://example.org/v1/upload",
        "http://example.org/v1/upload",
    )
    .unwrap();
    assert!(resp.is_new_project);
    assert_eq!(resp.status, 201);
}

// --- validate_endpoint_url_shape with DiscoveryRoot ---

#[test]
fn validate_discovery_root_rejects_non_http_scheme() {
    let url = Url::parse("ftp://example.org").unwrap();
    let err = validate_endpoint_url_shape(&url, EndpointKind::DiscoveryRoot).unwrap_err();
    assert!(matches!(err, PublishError::InvalidDiscoveryRoot { .. }));
}

#[test]
fn validate_discovery_root_rejects_upload_endpoint_path() {
    let url = Url::parse("https://example.org/v1/upload").unwrap();
    let err = validate_endpoint_url_shape(&url, EndpointKind::DiscoveryRoot).unwrap_err();
    assert!(matches!(err, PublishError::InvalidDiscoveryRoot { .. }));
}

#[test]
fn validate_discovery_root_rejects_query_and_fragment() {
    let url = Url::parse("https://example.org/index?x=1").unwrap();
    let err = validate_endpoint_url_shape(&url, EndpointKind::DiscoveryRoot).unwrap_err();
    assert!(matches!(err, PublishError::InvalidDiscoveryRoot { .. }));
}

// --- prepare_publish_payload error cases ---

mod prepare_publish {
    use crate::utils::sha256_lowercase_hex;

    use super::super::prepare_publish_payload;
    use super::PublishError;
    use camino::Utf8Path;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn deflate() -> SimpleFileOptions {
        SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::DEFAULT)
    }

    fn stored() -> SimpleFileOptions {
        SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .last_modified_time(zip::DateTime::DEFAULT)
    }

    /// Write a ZIP with the given entries to a NamedTempFile; keep the file
    /// alive by returning it alongside the path.
    fn write_zip(
        entries: &[(&str, &[u8], SimpleFileOptions)],
    ) -> (tempfile::NamedTempFile, camino::Utf8PathBuf) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = camino::Utf8PathBuf::from(tmp.path().to_str().unwrap());
        {
            let f = std::fs::File::create(tmp.path()).unwrap();
            let mut zip = zip::ZipWriter::new(f);
            for (name, content, opts) in entries {
                zip.start_file(*name, *opts).unwrap();
                zip.write_all(content).unwrap();
            }
            zip.finish().unwrap();
        }
        (tmp, path)
    }

    /// Minimal `.project.json` that passes all info-level checks up to the
    /// archive loop. Caller can override individual fields in the JSON.
    fn project_json(publisher: &str, name: &str, version: &str, license: &str) -> Vec<u8> {
        format!(
            r#"{{"name":"{name}","publisher":"{publisher}","version":"{version}","license":"{license}"}}"#
        )
        .into_bytes()
    }

    fn base_project() -> Vec<u8> {
        project_json("test-pub", "test-pkg", "1.0.0", "MIT")
    }

    /// `.meta.json` with a single source file, correct checksum, and valid
    /// SysML metamodel. `file_content` is the bytes that will be written to
    /// the archive for `file_name`.
    fn meta_json_with_file(file_name: &str, file_content: &[u8], symbol: &str) -> Vec<u8> {
        let cksum = sha256_lowercase_hex(file_content);
        format!(
            r#"{{"index":{{"{symbol}":"{file_name}"}},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201","checksum":{{"{file_name}":{{"value":"{cksum}","algorithm":"SHA256"}}}}}}"#
        )
        .into_bytes()
    }

    /// `.meta.json` with no source files (empty index, no checksum).
    fn meta_json_empty() -> Vec<u8> {
        br#"{"index":{},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201"}"#.to_vec()
    }

    /// Base entries that pass every check up to (but not including) the
    /// archive-file loop. Archive loop errors can be triggered by appending
    /// one more "bad" entry.
    fn pre_loop_entries() -> (Vec<u8>, Vec<u8>) {
        (base_project(), meta_json_empty())
    }

    /// Complete set of archive entries for a fully-valid kpar with one source
    /// file `test.sysml` containing `content`.
    fn valid_entries(sysml_content: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let meta = meta_json_with_file("test.sysml", sysml_content, "Test");
        (base_project(), meta)
    }

    // ── KparRead ────────────────────────────────────────────────────────────

    #[test]
    fn kpar_read_rejects_non_zip_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"this is not a zip file").unwrap();
        let path = Utf8Path::new(tmp.path().to_str().unwrap());
        let err = prepare_publish_payload(path).expect_err("expected Err");
        assert!(matches!(err, PublishError::KparRead(..)));
    }

    #[test]
    fn kpar_read_rejects_zip_without_project_json() {
        // A valid ZIP but containing no .project.json — guess_root fails.
        let (_tmp, path) = write_zip(&[("unrelated.txt", b"hello", deflate())]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::KparRead(..)));
    }

    // ── ProjectNotAtRoot ─────────────────────────────────────────────────────

    #[test]
    fn project_not_at_root() {
        // .project.json is inside a subdirectory; publish requires root placement.
        let (_tmp, path) =
            write_zip(&[("subdir/.project.json", base_project().as_slice(), deflate())]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::ProjectNotAtRoot { .. }));
    }

    // ── MissingMeta ──────────────────────────────────────────────────────────

    #[test]
    fn missing_meta() {
        let (_tmp, path) = write_zip(&[(".project.json", base_project().as_slice(), deflate())]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::MissingMeta));
    }

    // ── InfoMetaValidation ───────────────────────────────────────────────────

    #[test]
    fn info_meta_validation_project_bad_semver() {
        let bad_project = project_json("test-pub", "test-pkg", "not-semver", "MIT");
        let (_tmp, path) = write_zip(&[
            (".project.json", bad_project.as_slice(), deflate()),
            (".meta.json", meta_json_empty().as_slice(), deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(
            err,
            PublishError::InfoMetaValidation {
                name: "project",
                ..
            }
        ));
    }

    #[test]
    fn info_meta_validation_meta_bad_checksum_alg() {
        // Algorithm field is not one of the recognised values.
        let bad_meta = br#"{"index":{"Sym":"f.sysml"},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201","checksum":{"f.sysml":{"value":"aa","algorithm":"NOTAKNOWNALG"}}}"#;
        let (_tmp, path) = write_zip(&[
            (".project.json", base_project().as_slice(), deflate()),
            (".meta.json", bad_meta, deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(
            err,
            PublishError::InfoMetaValidation { name: "meta", .. }
        ));
    }

    // ── MissingPublisher ─────────────────────────────────────────────────────

    #[test]
    fn missing_publisher() {
        let no_pub = br#"{"name":"test-pkg","version":"1.0.0","license":"MIT"}"#;
        let (_tmp, path) = write_zip(&[
            (".project.json", no_pub, deflate()),
            (".meta.json", meta_json_empty().as_slice(), deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::MissingPublisher));
    }

    // ── InvalidPublisher ─────────────────────────────────────────────────────

    #[test]
    fn invalid_publisher() {
        let bad = project_json("bad__pub", "test-pkg", "1.0.0", "MIT");
        let (_tmp, path) = write_zip(&[
            (".project.json", bad.as_slice(), deflate()),
            (".meta.json", meta_json_empty().as_slice(), deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::InvalidPublisher(..)));
    }

    // ── InvalidName ──────────────────────────────────────────────────────────

    #[test]
    fn invalid_name() {
        let bad = project_json("test-pub", "bad__name", "1.0.0", "MIT");
        let (_tmp, path) = write_zip(&[
            (".project.json", bad.as_slice(), deflate()),
            (".meta.json", meta_json_empty().as_slice(), deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::InvalidName(..)));
    }

    // ── VersionBuildMetadata ─────────────────────────────────────────────────

    #[test]
    fn version_build_metadata() {
        let bad = project_json("test-pub", "test-pkg", "1.0.0+build", "MIT");
        let (_tmp, path) = write_zip(&[
            (".project.json", bad.as_slice(), deflate()),
            (".meta.json", meta_json_empty().as_slice(), deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::VersionBuildMetadata { .. }));
    }

    // ── MissingLicense ───────────────────────────────────────────────────────

    #[test]
    fn missing_license() {
        let no_lic = br#"{"name":"test-pkg","publisher":"test-pub","version":"1.0.0"}"#;
        let (_tmp, path) = write_zip(&[
            (".project.json", no_lic, deflate()),
            (".meta.json", meta_json_empty().as_slice(), deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::MissingLicense));
    }

    // ── InvalidLicense ───────────────────────────────────────────────────────

    #[test]
    fn invalid_license() {
        let bad = project_json("test-pub", "test-pkg", "1.0.0", "NOT-A-LICENSE!!!");
        let (_tmp, path) = write_zip(&[
            (".project.json", bad.as_slice(), deflate()),
            (".meta.json", meta_json_empty().as_slice(), deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::InvalidLicense { .. }));
    }

    // ── MissingMetamodel ─────────────────────────────────────────────────────

    #[test]
    fn missing_metamodel() {
        let no_meta = br#"{"index":{},"created":"2025-01-01T00:00:00Z"}"#;
        let (_tmp, path) = write_zip(&[
            (".project.json", base_project().as_slice(), deflate()),
            (".meta.json", no_meta, deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::MissingMetamodel));
    }

    // ── Archive-loop errors (ExecInArchive, UnsupportedCompression, Encrypted)

    // Each test below uses valid .project.json + .meta.json and appends one
    // "bad" file entry; the archive loop fires before any checksum check.

    #[test]
    fn exec_in_archive() {
        let (proj, meta) = pre_loop_entries();
        let exec = deflate().unix_permissions(0o100755);
        let (_tmp, path) = write_zip(&[
            (".project.json", proj.as_slice(), deflate()),
            (".meta.json", meta.as_slice(), deflate()),
            ("test.sysml", b"package Test;", exec),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::ExecInArchive { .. }));
    }

    #[test]
    fn unsupported_compression() {
        let (proj, meta) = pre_loop_entries();
        let (_tmp, path) = write_zip(&[
            (".project.json", proj.as_slice(), deflate()),
            (".meta.json", meta.as_slice(), deflate()),
            ("test.sysml", b"package Test;", stored()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(
            matches!(err, PublishError::UnsupportedCompression { .. }),
            "got: {err}"
        );
    }

    #[test]
    fn symlink_in_archive() {
        let (proj, meta) = pre_loop_entries();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = camino::Utf8PathBuf::from(tmp.path().to_str().unwrap());
        {
            let f = std::fs::File::create(tmp.path()).unwrap();
            let mut zip = zip::ZipWriter::new(f);
            zip.start_file(".project.json", deflate()).unwrap();
            zip.write_all(proj.as_slice()).unwrap();
            zip.start_file(".meta.json", deflate()).unwrap();
            zip.write_all(meta.as_slice()).unwrap();
            zip.add_symlink("test.sysml", "target-path", deflate())
                .unwrap();
            zip.finish().unwrap();
        }
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::Symlink { .. }));
    }

    #[test]
    fn encrypted_entry() {
        use zip::unstable::write::FileOptionsExt;
        let (proj, meta) = pre_loop_entries();
        let enc_opts = deflate().with_deprecated_encryption(b"secret").unwrap();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = camino::Utf8PathBuf::from(tmp.path().to_str().unwrap());
        {
            let f = std::fs::File::create(tmp.path()).unwrap();
            let mut zip = zip::ZipWriter::new(f);
            zip.start_file(".project.json", deflate()).unwrap();
            zip.write_all(proj.as_slice()).unwrap();
            zip.start_file(".meta.json", deflate()).unwrap();
            zip.write_all(meta.as_slice()).unwrap();
            zip.start_file("test.sysml", enc_opts).unwrap();
            zip.write_all(b"package Test;").unwrap();
            zip.finish().unwrap();
        }
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::Encrypted { .. }));
    }

    // ── DisallowedPath ───────────────────────────────────────────────────────

    #[test]
    fn disallowed_path_current_dir_prefix() {
        let (proj, meta) = pre_loop_entries();
        let (_tmp, path) = write_zip(&[
            (".project.json", proj.as_slice(), deflate()),
            (".meta.json", meta.as_slice(), deflate()),
            ("./test.sysml", b"package Test;", deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::DisallowedPath { .. }));
    }

    // ── MissingLicenseFile ───────────────────────────────────────────────────

    #[test]
    fn missing_license_file() {
        // Valid project + meta, archive passes file loop, but LICENSES/MIT.txt
        // is absent.
        let (proj, meta) = pre_loop_entries();
        let (_tmp, path) = write_zip(&[
            (".project.json", proj.as_slice(), deflate()),
            (".meta.json", meta.as_slice(), deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::MissingLicenseFile { .. }));
    }

    // ── MissingChecksum ──────────────────────────────────────────────────────

    #[test]
    fn missing_checksum() {
        // meta.json has no checksum field at all.
        let meta_no_cksum =
            br#"{"index":{},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201"}"#;
        let (_tmp, path) = write_zip(&[
            (".project.json", base_project().as_slice(), deflate()),
            (".meta.json", meta_no_cksum, deflate()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::MissingChecksum));
    }

    // ── EmptyChecksum ────────────────────────────────────────────────────────

    #[test]
    fn empty_checksum() {
        // checksum is present but empty.
        let meta_empty_cksum =
            br#"{"index":{},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201","checksum":{}}"#;
        let (_tmp, path) = write_zip(&[
            (".project.json", base_project().as_slice(), deflate()),
            (".meta.json", meta_empty_cksum, deflate()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::EmptyChecksum));
    }

    // ── IncorrectFileFormat ──────────────────────────────────────────────────

    #[test]
    fn incorrect_file_format() {
        // checksum references a .kerml file but metamodel is SysML.
        let cksum = sha256_lowercase_hex(b"content");
        let meta = format!(
            r#"{{"index":{{"Sym":"f.sysml"}},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201","checksum":{{"f.kerml":{{"value":"{cksum}","algorithm":"SHA256"}},"f.sysml":{{"value":"{cksum}","algorithm":"SHA256"}}}}}}"#
        );
        let (_tmp, path) = write_zip(&[
            (".project.json", base_project().as_slice(), deflate()),
            (".meta.json", meta.as_bytes(), deflate()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
            ("f.sysml", b"content", deflate()),
            ("f.kerml", b"content", deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::IncorrectFileFormat { .. }));
    }

    // ── UnsupportedFileChecksumType ──────────────────────────────────────────

    #[test]
    fn unsupported_file_checksum_type() {
        // SHA1 is a valid algorithm but not SHA256.
        let sha1_val = "a".repeat(40); // 40 hex chars = valid SHA1 length
        let meta = format!(
            r#"{{"index":{{"Sym":"f.sysml"}},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201","checksum":{{"f.sysml":{{"value":"{sha1_val}","algorithm":"SHA1"}}}}}}"#
        );
        let (_tmp, path) = write_zip(&[
            (".project.json", base_project().as_slice(), deflate()),
            (".meta.json", meta.as_bytes(), deflate()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(
            err,
            PublishError::UnsupportedFileChecksumType { .. }
        ));
    }

    // ── MissingFile ──────────────────────────────────────────────────────────

    #[test]
    fn missing_file() {
        // checksum mentions test.sysml but the archive does not contain it.
        let cksum = sha256_lowercase_hex(b"package Test;");
        let meta = format!(
            r#"{{"index":{{"Test":"test.sysml"}},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201","checksum":{{"test.sysml":{{"value":"{cksum}","algorithm":"SHA256"}}}}}}"#
        );
        let (_tmp, path) = write_zip(&[
            (".project.json", base_project().as_slice(), deflate()),
            (".meta.json", meta.as_bytes(), deflate()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
            // test.sysml intentionally omitted
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::MissingFile { .. }));
    }

    // ── IncorrectFileChecksum ────────────────────────────────────────────────

    #[test]
    fn incorrect_file_checksum() {
        let wrong_cksum = "f".repeat(64); // valid hex length but wrong value
        let meta = format!(
            r#"{{"index":{{"Test":"test.sysml"}},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201","checksum":{{"test.sysml":{{"value":"{wrong_cksum}","algorithm":"SHA256"}}}}}}"#
        );
        let (_tmp, path) = write_zip(&[
            (".project.json", base_project().as_slice(), deflate()),
            (".meta.json", meta.as_bytes(), deflate()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
            ("test.sysml", b"package Test;", deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::IncorrectFileChecksum { .. }));
    }

    // ── NonexistentSymbolExported ────────────────────────────────────────────

    #[test]
    fn nonexistent_symbol_exported() {
        // index claims "Ghost" is defined in test.sysml, but the file only
        // defines the package "Test".
        let content = b"package Test;";
        let cksum = sha256_lowercase_hex(content);
        let meta = format!(
            r#"{{"index":{{"Ghost":"test.sysml"}},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201","checksum":{{"test.sysml":{{"value":"{cksum}","algorithm":"SHA256"}}}}}}"#
        );
        let (_tmp, path) = write_zip(&[
            (".project.json", base_project().as_slice(), deflate()),
            (".meta.json", meta.as_bytes(), deflate()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
            ("test.sysml", content, deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(
            err,
            PublishError::NonexistentSymbolExported { .. }
        ));
    }

    // ── IndexFail ────────────────────────────────────────────────────────────

    #[test]
    fn index_fail() {
        // Garbage content in a .sysml file causes extract_symbols to fail.
        let content = b"@@@NOT VALID SYSML@@@";
        let cksum = sha256_lowercase_hex(content);
        let meta = format!(
            r#"{{"index":{{"Sym":"test.sysml"}},"created":"2025-01-01T00:00:00Z","metamodel":"https://www.omg.org/spec/SysML/20250201","checksum":{{"test.sysml":{{"value":"{cksum}","algorithm":"SHA256"}}}}}}"#
        );
        let (_tmp, path) = write_zip(&[
            (".project.json", base_project().as_slice(), deflate()),
            (".meta.json", meta.as_bytes(), deflate()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
            ("test.sysml", content, deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::IndexFail { .. }));
    }

    // ── UnexpectedFile ───────────────────────────────────────────────────────

    #[test]
    fn unexpected_file() {
        // A file that is neither in checksum nor a recognised ancillary file.
        let content = b"package Test;";
        let (proj, meta) = valid_entries(content);
        let (_tmp, path) = write_zip(&[
            (".project.json", proj.as_slice(), deflate()),
            (".meta.json", meta.as_slice(), deflate()),
            ("LICENSES/MIT.txt", b"MIT License", deflate()),
            ("test.sysml", content, deflate()),
            ("extra.txt", b"surprise", deflate()),
        ]);
        let err = prepare_publish_payload(&path).expect_err("expected Err");
        assert!(matches!(err, PublishError::UnexpectedFile { .. }));
    }
}
