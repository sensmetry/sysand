use std::collections::HashMap;

use indexmap::IndexMap;
use typed_path::Utf8UnixPath;

use crate::{
    model::{
        InterchangeProjectChecksumRaw, InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw,
        KerMlChecksumAlg,
    },
    project::{ProjectRead, hash_reader, memory::InMemoryProject},
};

#[test]
fn test_sanity_check_hasher() -> Result<(), Box<dyn std::error::Error>> {
    let input = "FooBarBaz";

    // echo -n "FooBarBaz" | sha256sum
    assert_eq!(
        format!("{:x}", hash_reader(&mut std::io::Cursor::new(input))?),
        "4da8b89a905445e96dd0ab6c9be9a72c8b0ffc686a57a3cc6808a8952a3560ed"
    );

    Ok(())
}

#[test]
fn test_canonicalization_no_checksums() -> Result<(), Box<dyn std::error::Error>> {
    let project = InMemoryProject {
        info: Some(InterchangeProjectInfoRaw {
            name: "test_canonicalization".to_string(),
            publisher: None,
            description: None,
            version: "1.2.3".to_string(),
            license: None,
            maintainer: vec![],
            website: None,
            topic: vec![],
            usage: vec![],
        }),
        meta: Some(InterchangeProjectMetadataRaw {
            index: IndexMap::default(),
            created: "123".to_string(),
            metamodel: None,
            includes_derived: None,
            includes_implied: None,
            checksum: Some(IndexMap::from([(
                "MyFile.txt".to_string(),
                InterchangeProjectChecksumRaw {
                    algorithm: KerMlChecksumAlg::None.to_string(),
                    value: "".to_string(),
                },
            )])),
        }),
        files: HashMap::from([(
            Utf8UnixPath::new("MyFile.txt").to_path_buf(),
            "FooBarBaz".to_string(),
        )]),
        nominal_sources: vec![],
    };

    let Some(canonical_info) = project.canonical_meta()? else {
        panic!()
    };

    let Some(checksums) = canonical_info.checksum else {
        panic!()
    };

    assert_eq!(checksums.len(), 1);
    assert_eq!(
        checksums.get("MyFile.txt"),
        Some(&InterchangeProjectChecksumRaw {
            value: "4da8b89a905445e96dd0ab6c9be9a72c8b0ffc686a57a3cc6808a8952a3560ed".to_string(),
            algorithm: KerMlChecksumAlg::Sha256.to_string()
        })
    );

    Ok(())
}
