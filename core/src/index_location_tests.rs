// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::assert_matches;

use super::*;

const GITLAB_TEMPLATE: &str =
    "https://gitlab.com/api/v4/projects/123/repository/files/{path}/raw?ref=main";

fn parse_template(s: &str) -> IndexUrlTemplate {
    match IndexLocation::parse(s).unwrap() {
        IndexLocation::Template(template) => template,
        IndexLocation::Root(url) => panic!("expected template, got root `{url}`"),
    }
}

#[test]
fn plain_url_parses_as_root_with_trailing_slash() {
    // The trailing slash is an invariant of `Root`, established at parse
    // time so `resolve` can join without renormalizing.
    let location = IndexLocation::parse("https://example.org/index").unwrap();
    assert_eq!(
        location,
        IndexLocation::Root(url::Url::parse("https://example.org/index/").unwrap())
    );
}

#[test]
fn relative_url_is_rejected() {
    assert_matches!(
        IndexLocation::parse("example.org/index"),
        Err(IndexLocationError::RelativeUrl { .. })
    );
}

#[test]
fn braces_dispatch_to_template() {
    assert_matches!(
        IndexLocation::parse(GITLAB_TEMPLATE),
        Ok(IndexLocation::Template(_))
    );
}

#[test]
fn template_expands_with_full_percent_encoding() {
    let template = parse_template(GITLAB_TEMPLATE);
    let url = template.expand("admin/proj0/4.0.20/project.kpar".split('/'));
    assert_eq!(
        url.as_str(),
        "https://gitlab.com/api/v4/projects/123/repository/files/\
         admin%2Fproj0%2F4.0.20%2Fproject.kpar/raw?ref=main"
    );
}

#[test]
fn template_encodes_non_unreserved_bytes() {
    let template = parse_template("https://example.org/files/{path}?ref=main");
    let url = template.expand("a b/+%/ö.json".split('/'));
    assert_eq!(
        url.as_str(),
        "https://example.org/files/a%20b%2F%2B%25%2F%C3%B6.json?ref=main"
    );
}

#[test]
fn template_preserves_unreserved_bytes() {
    let template = parse_template("https://example.org/files/{path}");
    let url = template.expand("Az0-9_.~x".split('/'));
    assert_eq!(url.as_str(), "https://example.org/files/Az0-9_.~x");
}

#[test]
fn template_placeholder_in_query_expands() {
    let template = parse_template("https://example.org/get?file={path}&ref=main");
    let url = template.expand("a/b.json".split('/'));
    assert_eq!(
        url.as_str(),
        "https://example.org/get?file=a%2Fb.json&ref=main"
    );
}

#[test]
fn raw_template_keeps_slashes_literal() {
    let template =
        parse_template("https://gitea.example.org/api/v1/repos/o/r/raw/{path_raw}?ref=main");
    let url = template.expand("admin/proj0/versions.json".split('/'));
    assert_eq!(
        url.as_str(),
        "https://gitea.example.org/api/v1/repos/o/r/raw/admin/proj0/versions.json?ref=main"
    );
}

#[test]
fn raw_template_encodes_within_segments() {
    let template = parse_template("https://example.org/raw/{path_raw}");
    let url = template.expand("a b/ö/c+d.json".split('/'));
    assert_eq!(
        url.as_str(),
        "https://example.org/raw/a%20b/%C3%B6/c%2Bd.json"
    );
}

#[test]
fn mixing_both_placeholders_is_rejected() {
    assert_matches!(
        IndexLocation::parse("https://example.org/{path}/{path_raw}"),
        Err(IndexLocationError::PlaceholderCount { count: 2, .. })
    );
}

#[test]
fn case_typo_raw_placeholder_gets_hint() {
    let error = IndexLocation::parse("https://example.org/{PATH_RAW}/x").unwrap_err();
    assert!(error.to_string().contains("did you mean `{path_raw}`?"));
}

#[test]
fn resolve_on_root_matches_append_semantics() {
    let location = IndexLocation::parse("https://example.org/index").unwrap();
    let url = location.resolve("admin/proj0/versions.json".split('/'));
    assert_eq!(
        url.as_str(),
        "https://example.org/index/admin/proj0/versions.json"
    );
    // Trailing slash on the root makes no difference.
    let location = IndexLocation::parse("https://example.org/index/").unwrap();
    let url = location.resolve("admin/proj0/versions.json".split('/'));
    assert_eq!(
        url.as_str(),
        "https://example.org/index/admin/proj0/versions.json"
    );
}

#[test]
fn unknown_placeholder_is_rejected() {
    let error = IndexLocation::parse("https://example.org/{file}/raw").unwrap_err();
    assert_matches!(&error,
    IndexLocationError::UnknownPlaceholder { placeholder, .. } if placeholder.as_ref() == "{file}");
}

#[test]
fn case_typo_placeholder_gets_hint() {
    let error = IndexLocation::parse("https://example.org/{Path}/raw").unwrap_err();
    let message = error.to_string();
    assert!(message.contains("did you mean `{path}`?"), "{message}");
}

#[test]
fn missing_placeholder_with_braces_is_rejected() {
    assert_matches!(
        IndexLocation::parse("https://example.org/files/{path"),
        Err(IndexLocationError::StrayBrace { .. })
    );
    assert_matches!(
        IndexLocation::parse("https://example.org/files/path}"),
        Err(IndexLocationError::StrayBrace { .. })
    );
}

#[test]
fn duplicate_placeholder_is_rejected() {
    assert_matches!(
        IndexLocation::parse("https://example.org/{path}/{path}"),
        Err(IndexLocationError::PlaceholderCount { count: 2, .. })
    );
}

#[test]
fn pre_encoded_placeholder_is_rejected_with_hint() {
    let error =
        IndexLocation::parse("https://example.org/files/%7Bpath%7D/raw?ref=main").unwrap_err();
    assert_matches!(&error, IndexLocationError::PreEncodedPlaceholder { .. });
    assert!(error.to_string().contains("{path}"));
}

#[test]
fn encoded_braces_that_are_not_placeholders_are_accepted() {
    let location = IndexLocation::parse("https://example.org/dir%7Bx/idx").unwrap();
    assert_matches!(location, IndexLocation::Root(_));
}

#[test]
fn non_http_root_is_rejected_at_parse() {
    // A non-hierarchical scheme (`mailto:`) cannot anchor relative index
    // paths; `IndexLocation` rejects it at construction rather than
    // deferring to resolution.
    assert_matches!(
        IndexLocation::parse("mailto:foo@example.org"),
        Err(IndexLocationError::UnsupportedScheme { .. })
    );
}

#[test]
fn template_fragment_is_rejected() {
    assert_matches!(
        IndexLocation::parse("https://example.org/files/{path}#frag"),
        Err(IndexLocationError::Fragment { .. })
    );
}

#[test]
fn root_fragment_is_rejected() {
    // Same invariant as templates: a fragment is never sent to the
    // server, so on an index location it is always a mistake.
    assert_matches!(
        IndexLocation::parse("https://example.org/idx#frag"),
        Err(IndexLocationError::Fragment { .. })
    );
}

#[test]
fn root_userinfo_is_rejected() {
    assert_matches!(
        IndexLocation::parse("https://user:pass@example.org/idx"),
        Err(IndexLocationError::Userinfo { .. })
    );
}

#[test]
fn parse_errors_never_echo_an_embedded_password() {
    // One URL per error branch that can carry userinfo: the userinfo
    // rejection, the scheme rejection, the parse failure (a space in
    // the host), and the template expansion failure. These messages
    // reach stderr and CI logs, so the password must never appear.
    for url in [
        "https://user:hunter2@example.com/idx",
        "ftp://user:hunter2@example.com/idx",
        "https://user:hunter2@exa mple.com/idx",
        "https://user:hunter2@example.com/files/{path}",
    ] {
        let message = IndexLocation::parse(url).unwrap_err().to_string();
        assert!(
            !message.contains("hunter2") && !message.contains("user:"),
            "url {url:?} leaked userinfo: {message}"
        );
        assert!(
            message.contains("<redacted>@"),
            "url {url:?} must show the redaction marker: {message}"
        );
    }
}

#[test]
fn template_in_host_is_rejected() {
    assert_matches!(
        IndexLocation::parse("https://{path}.example.org/files"),
        Err(IndexLocationError::InvalidTemplate { .. })
    );
}

#[test]
fn template_non_http_scheme_is_rejected() {
    assert_matches!(
        IndexLocation::parse("ftp://example.org/files/{path}"),
        Err(IndexLocationError::UnsupportedScheme { .. })
    );
}

#[test]
fn template_userinfo_is_rejected() {
    assert_matches!(
        IndexLocation::parse("https://user:pass@example.org/files/{path}"),
        Err(IndexLocationError::Userinfo { .. })
    );
}

#[test]
fn schemeless_template_is_rejected_as_relative() {
    assert_matches!(
        IndexLocation::parse("example.org/files/{path}"),
        Err(IndexLocationError::RelativeTemplate { .. })
    );
}

#[test]
fn template_prefix_normalizes_at_parse() {
    // The literal prefix is rewritten to the text `url::Url` serializes,
    // so `Display` renders one canonical spelling per template; the
    // placeholder and suffix stay verbatim.
    for (spelled, canonical) in [
        // Host case and the default port.
        (
            "HTTPS://GitLab.COM:443/files/{path}/raw?ref=main",
            "https://gitlab.com/files/{path}/raw?ref=main",
        ),
        // A path-less prefix gains the explicit root `/`.
        (
            "https://example.com?x={path}",
            "https://example.com/?x={path}",
        ),
        // Mid-segment placeholder: the cut lands exactly where the
        // substituted text will begin.
        (
            "https://Example.com/files/v{path}.json",
            "https://example.com/files/v{path}.json",
        ),
        // IDN host to punycode.
        (
            "https://bücher.example/{path_raw}",
            "https://xn--bcher-kva.example/{path_raw}",
        ),
        // Literal text aliasing the internal probe's encoding does not
        // confuse the cut.
        (
            "https://example.com/pr%20obe/{path}?x=pr%20obe",
            "https://example.com/pr%20obe/{path}?x=pr%20obe",
        ),
        // The suffix stays as written, whatever it contains.
        (
            "http://EXAMPLE.com:80/a/{path}/../x?Y=Z",
            "http://example.com/a/{path}/../x?Y=Z",
        ),
        // A prefix ending in `..` or `.` is mid-segment text in every
        // expansion (`..{path}` names a `..foo` segment), so it must
        // survive normalization instead of resolving as a dot segment.
        (
            "https://example.com/a/..{path}",
            "https://example.com/a/..{path}",
        ),
        (
            "https://example.com/a/.{path}.json",
            "https://example.com/a/.{path}.json",
        ),
        // A space before the placeholder is likewise mid-segment: it
        // normalizes to `%20` rather than being trimmed as trailing
        // whitespace.
        (
            "https://example.com/a {path}",
            "https://example.com/a%20{path}",
        ),
    ] {
        let location = IndexLocation::parse(spelled).unwrap();
        assert_eq!(location.to_string(), canonical, "for `{spelled}`");
        // Idempotent: parsing the canonical text reproduces it.
        assert_eq!(
            IndexLocation::parse(canonical).unwrap().to_string(),
            canonical,
            "reparsing the canonical form of `{spelled}`"
        );
    }
}

#[test]
fn normalized_template_expands_like_the_spelled_one() {
    // Prefix normalization must not change where requests go: the
    // spelled and canonical forms expand to the same URL.
    let spelled = parse_template("HTTPS://Example.COM:443/files/{path}?ref=main");
    let canonical = parse_template("https://example.com/files/{path}?ref=main");
    assert_eq!(
        spelled.expand(["a b", "c.json"]),
        canonical.expand(["a b", "c.json"])
    );
    assert_eq!(
        spelled.expand(["a b", "c.json"]).as_str(),
        "https://example.com/files/a%20b%2Fc.json?ref=main"
    );
}

#[test]
fn display_round_trips_templates_and_normalizes_roots() {
    assert_eq!(
        IndexLocation::parse(GITLAB_TEMPLATE).unwrap().to_string(),
        GITLAB_TEMPLATE
    );
    assert_eq!(
        IndexLocation::parse("https://example.org/index")
            .unwrap()
            .to_string(),
        "https://example.org/index/"
    );
    // A root query is kept; the trailing slash lands on the path, so
    // resolved index paths append before the query.
    assert_eq!(
        IndexLocation::parse("HTTPS://Example.ORG:443/index?ref=main")
            .unwrap()
            .to_string(),
        "https://example.org/index/?ref=main"
    );
}
