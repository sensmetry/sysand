// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::auth::{GlobMapBuilder, GlobMapResultMut};

#[test]
fn basic_globmap_lookup() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = GlobMapBuilder::new();
    builder.add("a*.com/*", 1);
    builder.add("a*.com/**", 2);
    builder.add("b.com/*", 3);
    builder.add("a*.com/*/*", 4);
    let mut globmap = builder.build()?;

    if let GlobMapResultMut::Ambiguous(vals) = globmap.lookup_mut("axx.com/xxx") {
        let vals: Vec<i32> = vals.into_iter().map(|(_, i)| *i).collect();
        assert_eq!(vals, vec![1, 2]);
    } else {
        panic!("Expected ambiguous result.");
    }

    if let GlobMapResultMut::Ambiguous(vals) = globmap.lookup_mut("axx.com/xxx/xxx") {
        let vals: Vec<i32> = vals.into_iter().map(|(_, i)| *i).collect();
        assert_eq!(vals, vec![2, 4]);
    } else {
        panic!("Expected ambiguous result.");
    }

    let key = "axx.com/xxx/xxx/xxx";
    if let GlobMapResultMut::Found(k, val) = globmap.lookup_mut(key) {
        assert_eq!(k, key);
        assert_eq!(*val, 2);
    } else {
        panic!("Expected unambiguous result.");
    }

    let key = "b.com/xxx";
    if let GlobMapResultMut::Found(k, val) = globmap.lookup_mut(key) {
        assert_eq!(k, key);
        assert_eq!(*val, 3);
    } else {
        panic!("Expected unambiguous result.");
    }

    if let GlobMapResultMut::NotFound = globmap.lookup_mut("axx.com") {
    } else {
        panic!("Expected no result.");
    }

    if let GlobMapResultMut::NotFound = globmap.lookup_mut("bxx.com/xxx") {
    } else {
        panic!("Expected no result.");
    }

    if let GlobMapResultMut::NotFound = globmap.lookup_mut("cxx.com/xxx") {
    } else {
        panic!("Expected no result.");
    }

    Ok(())
}

#[test]
fn globmap_matches_template_expanded_urls() -> Result<(), Box<dyn std::error::Error>> {
    // The credential glob a user configures for a templated index
    // (`SYSAND_CRED_X=https://gitlab.com/api/v4/projects/123/**`) must
    // match the expanded request URLs, where the file path sits mid-URL
    // percent-encoded and a query string follows.
    // This glob only fits the *expanded* request URL: it matches on the
    // percent-encoded file path (`admin%2F...`), which does not exist in
    // the raw template (where the placeholder still reads `{path}`).
    let mut builder = GlobMapBuilder::new();
    builder.add(
        "https://gitlab.com/api/v4/projects/123/repository/files/admin%2F*/raw?ref=main",
        1,
    );
    let mut globmap = builder.build()?;

    let template = crate::index_location::IndexLocation::parse(
        "https://gitlab.com/api/v4/projects/123/repository/files/{path}/raw?ref=main",
    )?;
    let expanded = template.resolve("admin/proj0/versions.json".split('/'));
    assert_eq!(
        expanded.as_str(),
        "https://gitlab.com/api/v4/projects/123/repository/files/\
         admin%2Fproj0%2Fversions.json/raw?ref=main"
    );

    // The raw template does not match the glob...
    assert!(matches!(
        globmap.lookup_mut(&template.to_string()),
        GlobMapResultMut::NotFound
    ));
    // ...but the expanded request URL does.
    if let GlobMapResultMut::Found(_, val) = globmap.lookup_mut(expanded.as_str()) {
        assert_eq!(*val, 1);
    } else {
        panic!("expected credential glob to match expanded template URL");
    }

    Ok(())
}

#[test]
fn publish_bearer_auth_map_keeps_bearer_drops_basic() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = crate::auth::StandardHTTPAuthenticationBuilder::new();
    builder.add_basic_auth("https://basic.example.com/*", "user", "password");
    builder.add_bearer_auth("https://bearer.example.com/*", "tok");
    let policy = builder.build()?;

    // By-ref extraction: the policy stays usable afterwards.
    let bearer_map = policy.publish_bearer_auth_map()?;

    if let crate::auth::GlobMapResult::Found(_, auth) =
        bearer_map.lookup("https://bearer.example.com/upload")
    {
        assert_eq!(&*auth.0, "tok");
    } else {
        panic!("expected bearer entry to be extracted");
    }

    assert!(matches!(
        bearer_map.lookup("https://basic.example.com/upload"),
        crate::auth::GlobMapResult::NotFound
    ));
    assert!(matches!(
        bearer_map.lookup("https://other.example.com/upload"),
        crate::auth::GlobMapResult::NotFound
    ));

    Ok(())
}
