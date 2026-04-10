use camino::Utf8PathBuf;
pub use httpmock;
use httpmock::{
    Method::{GET, HEAD}, Mock, MockServer, Then, When
};
use indexmap::{IndexMap, map::Entry};
use std::{collections::HashMap, fs, io::Write};
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};
use urlencoding::encode;
use zip::write::SimpleFileOptions;

use crate::{
    include::do_include,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw, InterchangeProjectUsageG},
    project::ProjectMut,
    project::memory::InMemoryProject,
};

// pub type ProjectMock = InMemoryProject;

// Use this instead of InMemoryProject to allow malformed .project.json and .meta.json
pub struct ProjectMock {
    pub all_files: HashMap<Utf8PathBuf, String>,
}

pub struct ProjectMockBuilder {
    pub in_memory_project: InMemoryProject,
}

fn into<T>(option: Option<impl Into<T>>) -> Option<T> {
    option.map(|value| value.into())
}

impl ProjectMockBuilder {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            in_memory_project: InMemoryProject::from_info_meta(
                InterchangeProjectInfoRaw::minimal(name.into(), version.into()),
                InterchangeProjectMetadataRaw {
                    index: IndexMap::default(),
                    created: chrono::Utc::now().to_rfc3339(),
                    metamodel: None,
                    includes_derived: None,
                    includes_implied: None,
                    checksum: None,
                },
            ),
        }
    }

    pub fn info(self: &Self) -> &InterchangeProjectInfoRaw {
        self.in_memory_project.info.as_ref().unwrap()
    }

    pub fn info_mut(self: &mut Self) -> &mut InterchangeProjectInfoRaw {
        self.in_memory_project.info.as_mut().unwrap()
    }

    pub fn meta(self: &Self) -> &InterchangeProjectMetadataRaw {
        self.in_memory_project.meta.as_ref().unwrap()
    }

    pub fn meta_mut(self: &mut Self) -> &mut InterchangeProjectMetadataRaw {
        self.in_memory_project.meta.as_mut().unwrap()
    }

    pub fn files(self: &Self) -> &HashMap<Utf8UnixPathBuf, String> {
        &self.in_memory_project.files
    }

    pub fn files_mut(self: &mut Self) -> &mut HashMap<Utf8UnixPathBuf, String> {
        &mut self.in_memory_project.files
    }

    pub fn with_description(self: &mut Self, description: Option<impl Into<String>>) -> &mut Self {
        self.info_mut().description = into(description);
        self
    }

    pub fn with_license(self: &mut Self, license: Option<impl Into<String>>) -> &mut Self {
        self.info_mut().license = into(license);
        self
    }

    pub fn with_maintainer(
        self: &mut Self,
        maintainer: impl IntoIterator<Item = impl Into<String>>,
    ) -> &mut Self {
        self.info_mut().maintainer = maintainer.into_iter().map(|m| m.into()).collect();
        self
    }

    pub fn with_website(self: &mut Self, website: Option<impl Into<String>>) -> &mut Self {
        self.info_mut().website = into(website);
        self
    }

    pub fn with_topic(
        self: &mut Self,
        topic: impl IntoIterator<Item = impl Into<String>>,
    ) -> &mut Self {
        self.info_mut().topic = topic.into_iter().map(|t| t.into()).collect();
        self
    }

    pub fn with_usage(
        self: &mut Self,
        usage: impl IntoIterator<Item = (impl Into<String>, Option<impl Into<String>>)>,
    ) -> &mut Self {
        self.info_mut()
            .usage
            .extend(
                usage
                    .into_iter()
                    .map(|(dep, ver)| InterchangeProjectUsageG {
                        resource: dep.into(),
                        version_constraint: into(ver),
                    }),
            );
        self
    }

    pub fn with_created(self: &mut Self, created: impl Into<String>) -> &mut Self {
        self.meta_mut().created = created.into();
        self
    }

    pub fn with_metamodel(self: &mut Self, metamodel: Option<impl Into<String>>) -> &mut Self {
        self.meta_mut().metamodel = into(metamodel);
        self
    }

    pub fn with_includes_derived(self: &mut Self, includes_derived: Option<bool>) -> &mut Self {
        self.meta_mut().includes_derived = includes_derived;
        self
    }

    pub fn with_includes_implied(self: &mut Self, includes_implied: Option<bool>) -> &mut Self {
        self.meta_mut().includes_implied = includes_implied;
        self
    }

    pub fn with_index_create_files(
        self: &mut Self,
        index: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
        compute_checksum: bool,
    ) -> &mut Self {
        for (symbol, path) in index.into_iter() {
            let symbol = symbol.into();
            let path = path.into();
            let index_entry = self.meta_mut().index.entry(symbol.clone());
            if let Entry::Occupied(index_entry) = index_entry {
                panic!("Index entry with key {} already exists", index_entry.key());
            }
            index_entry.insert_entry(path.clone());
            let files_entry = self.files_mut().entry(path.clone().into());
            let file_content = files_entry.or_insert(String::new());
            if !file_content.is_empty() && !file_content.ends_with('\n') {
                file_content.push('\n');
            }
            file_content.push_str(&format!("package '{symbol}'\n"));
            self.in_memory_project
                .include_source(Utf8UnixPath::new(&path), compute_checksum, true)
                .unwrap();
        }
        self
    }

    pub fn with_files<'a>(
        self: &mut Self,
        files: impl IntoIterator<Item = (&'a str, impl Into<String> + 'a)>,
        compute_checksum: bool,
        index_symbols: bool,
    ) -> &mut Self {
        files.into_iter().for_each(|(path, content)| {
            self.files_mut().insert(path.into(), content.into());
            do_include(
                &mut self.in_memory_project,
                Utf8UnixPath::new(path),
                compute_checksum,
                index_symbols,
                None,
            )
            .unwrap()
        });

        self
    }

    // pub fn compute_checksum(self: &mut Self) -> &mut Self {
    //     self.meta_mut()
    //         .add_checksum(path, algorithm, value, overwrite)
    // }

    pub fn build(self: &Self) -> ProjectMock {
        ProjectMock::new_raw(
            [
                (
                    &".project.json".into(),
                    &serde_json::to_string(&self.info()).unwrap(),
                ),
                (
                    &".meta.json".into(),
                    &serde_json::to_string(&self.meta()).unwrap(),
                ),
            ]
            .into_iter()
            .chain(self.files().iter()),
        )
    }

    // pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
    //     Self {
    //         project_info: InterchangeProjectInfoRaw::minimal(name.into(), version.into()),
    //         project_metadata: InterchangeProjectMetadataRaw::generate_blank(),
    //         other_files: HashMap::new(),
    //     }
    // }

    // pub fn with_created(self: &mut Self, created: impl Into<String>) -> &mut Self {
    //     self.project_metadata.created = created.into();
    //     self
    // }

    // pub fn with_usage(
    //     self: &mut Self,
    //     usage: impl IntoIterator<Item = (impl Into<String>, Option<impl Into<String>>)>,
    // ) -> &mut Self {
    //     self.project_info
    //         .usage
    //         .extend(
    //             usage
    //                 .into_iter()
    //                 .map(|(dep, ver)| InterchangeProjectUsageG {
    //                     resource: dep.into(),
    //                     version_constraint: ver.map(|val| val.into()),
    //                 }),
    //         );
    //     self
    // }

    // pub fn with_index_create_files(
    //     self: &mut Self,
    //     index: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    // ) -> &mut Self {
    //     for (symbol, file_path) in index.into_iter() {
    //         // let file_path_buf = Utf8PathBuf::from(file_path);
    //         let symbol = symbol.into();
    //         let file_path = file_path.into();
    //         let index_entry = self.project_metadata.index.entry(symbol.clone());
    //         if let Entry::Occupied(index_entry) = index_entry {
    //             panic!("Index entry with key {} already exists", index_entry.key());
    //         }
    //         index_entry.insert_entry(file_path.clone());
    //         let files_entry = self.other_files.entry(file_path.into());
    //         let file_content = files_entry.or_insert(String::new());
    //         if !file_content.is_empty() && !file_content.ends_with('\n') {
    //             file_content.push('\n');
    //         }
    //         file_content.push_str(&format!("package '{symbol}'\n"));
    //     }
    //     self
    // }

    // pub fn with_files<'a>(
    //     self: &mut Self,
    //     files: impl IntoIterator<Item = (&'a str, &'a str)>,
    // ) -> &mut Self {
    //     self.other_files.extend(
    //         files
    //             .into_iter()
    //             .map(|(path, content)| (path.to_string(), content.to_string())),
    //     );
    //     self
    // }

    // pub fn build(self: &Self) -> ProjectMock {
    //     ProjectMock::new_raw(
    //         [
    //             (
    //                 &".project.json".to_string(),
    //                 &serde_json::to_string(&self.project_info).unwrap(),
    //             ),
    //             (
    //                 &".meta.json".to_string(),
    //                 &serde_json::to_string(&self.project_metadata).unwrap(),
    //             ),
    //         ]
    //         .into_iter()
    //         .chain(self.other_files.iter()),
    //     )
    // }

    // pub fn build_to_folder(self: &Self, folder: &str) -> ProjectMock {}
}

// pub struct ProjectInfoMock {
//     name: String,
//     version: String,
//     usage: Vec<(String, String)>,
// }

// pub struct ProjectMetaMock {
//     index: IndexMap<String, String>,
//     created: String,
//     checksum: IndexMap<String, String>,
// }

// pub struct ProjectOverHttpMock {
//     bla: Mock,
// }

impl ProjectMock {
    // pub fn new_raw<'a>(files: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
    //     Self {
    //         files: files
    //             .into_iter()
    //             .map(|(path, content)| (Utf8PathBuf::from(path), content.to_string()))
    //             .collect(),
    //     }
    // }

    pub fn new_raw(
        files: impl IntoIterator<Item = (impl Into<Utf8PathBuf>, impl Into<String>)>,
    ) -> Self {
        Self {
            all_files: files
                .into_iter()
                .map(|(path, content)| (path.into(), content.into()))
                .collect(),
        }
    }

    pub fn builder(name: impl Into<String>, version: impl Into<String>) -> ProjectMockBuilder {
        ProjectMockBuilder::new(name, version)
    }

    pub fn new_small_example() -> Self {
        Self::builder("Lib test", "0.0.1")
            .with_index_create_files(
                [
                    ("Foo", "extras/foo.sysml"),
                    ("LibTest", "libtest.sysml"),
                    ("Baz", "extras/bar/baz.sysml"),
                ],
                true,
            )
            .build()

        // Self::new_high_level(
        //     "Lib test",
        //     "0.0.1",
        //     &[],
        //     &[
        //         ("Foo", "extras/foo.sysml"),
        //         ("LibTest", "libtest.sysml"),
        //         ("Baz", "extras/bar/baz.sysml"),
        //     ],
        // )

        // Self::new(
        //     InterchangeProjectInfoRaw::minimal("Lib test".to_string(), "0.0.1".to_string()),
        //     InterchangeProjectMetadataRaw {
        //         index: [
        //             ("Foo".to_string(), "extras/foo.sysml".to_string()),
        //             ("LibTest".to_string(), "libtest.sysml".to_string()),
        //             ("Baz".to_string(), "extras/bar/baz.sysml".to_string()),
        //         ]
        //         .into_iter()
        //         .collect(),
        //         ..InterchangeProjectMetadataRaw::generate_blank()
        //     },
        //     &[
        //         (
        //             ".project.json",
        //             r#"{"name": "Lib test","version": "0.0.1","usage": []}"#,
        //         ),
        //         (
        //             ".meta.json",
        //             r#"{"index":{"Foo":"extras/foo.sysml","LibTest":"libtest.sysml","Baz":"extras/bar/baz.sysml"},"created":"2025-05-30T12:34:24.977672Z"}"#,
        //         ),
        //         (
        //             "libtest.sysml",
        //             r#"package LibTest { attribute desc = "Just testing"; }"#,
        //         ),
        //         (
        //             "extras/foo.sysml",
        //             r#"package Foo { attribute desc = "More foo."; }"#,
        //         ),
        //         (
        //             "extras/bar/baz.sysml",
        //             r#"package Baz { attribute desc = "Bar Baz!"; }"#,
        //         ),
        //     ],
        // )

        // Self::new_raw(&[
        //     (
        //         ".project.json",
        //         r#"{"name": "Lib test","version": "0.0.1","usage": []}"#,
        //     ),
        //     (
        //         ".meta.json",
        //         r#"{"index":{"Foo":"extras/foo.sysml","LibTest":"libtest.sysml","Baz":"extras/bar/baz.sysml"},"created":"2025-05-30T12:34:24.977672Z"}"#,
        //     ),
        //     (
        //         "libtest.sysml",
        //         r#"package LibTest { attribute desc = "Just testing"; }"#,
        //     ),
        //     (
        //         "extras/foo.sysml",
        //         r#"package Foo { attribute desc = "More foo."; }"#,
        //     ),
        //     (
        //         "extras/bar/baz.sysml",
        //         r#"package Baz { attribute desc = "Bar Baz!"; }"#,
        //     ),
        // ])
    }

    // // TODO: perhaps add a callback to potentially modify the paths? E.g. add auth, change return code, etc.
    // pub fn add_to_mock_server(self: &Self, server: &mut ServerGuard) -> ProjectOverHttpMock {
    //     todo!();
    // }

    pub fn save_to_folder(self: &Self, root_path: &Utf8PathBuf) {
        for (path, contents) in self.all_files.iter() {
            let full_path: Utf8PathBuf = [root_path, path].iter().collect();
            fs::write(full_path, contents).unwrap();
        }
    }

    pub fn to_zip(
        self: &Self,
        options: SimpleFileOptions,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut cursor = std::io::Cursor::new(vec![]);
        let mut zip = zip::ZipWriter::new(&mut cursor);

        // let options = zip::write::SimpleFileOptions::default()
        //     .compression_method(zip::CompressionMethod::Stored)
        //     .unix_permissions(0o755);

        for (file_name, file_contents) in self.all_files.iter() {
            zip.start_file(file_name, options)?;
            zip.write_all(file_contents.as_bytes())?;
        }
        zip.finish().unwrap();

        cursor.flush()?;
        Ok(cursor.into_inner())
    }

    fn path_to_url_path(path: &Utf8PathBuf) -> String {
        let mut url_path = String::new();
        for component in path.components() {
            url_path += "/";
            url_path += &encode(component.as_str());
        }
        return url_path;
        // path.components()
        //     .map(|c| encode(c.as_str()).to_string())
        //     .join("/");
    }

    // fn get_request_file_path(req: &HttpMockRequest) -> Utf8PathBuf {
    //     let uri = req.uri();
    //     let path = uri.path();
    //     dbg!(path);
    //     Utf8PathBuf::from(&path[1..])
    // }

    pub fn add_to_server<'a>(&self, server: &'a MockServer, mut when_fn: impl FnMut(When) -> When, mut then_fn: impl FnMut(Then) -> Then) -> HashMap<Utf8PathBuf, Mock<'a>> {
        self.all_files
            .iter()
            .map(|(path, content)| {
                // for (path, content) in self.all_files.iter() {
                let mock = server.mock(|when, then| {
                    // println!("{}", Self::path_to_url_path(path));
                    when.and(&mut when_fn).method(GET)
                        // .method(HEAD)
                        .path(Self::path_to_url_path(path));
                    // .path(["/", path.as_str()].concat());
                    let content_type = if path.ends_with(".json") {
                        "application/json"
                    } else {
                        "text/plain"
                    };
                    then.and(&mut then_fn).status(200)
                        .header("content-type", content_type)
                        .body(content);
                });
                (path.into(), mock)
            })
            .collect()

        // let all_files1 = self.all_files.clone();
        // let all_files2 = self.all_files.clone();

        // let mock = server.mock(move |when, then| {
        //     when.is_true(move |req| {
        //         matches!(req.method(), Method::HEAD | Method::GET)
        //             && all_files1.contains_key(&Self::get_request_file_path(req))
        //     });
        //     then.respond_with(move |req| {
        //         let path = Self::get_request_file_path(req);
        //         let body = all_files2[&path].clone();
        //         let content_type = if path.ends_with(".json") {
        //             "application/json"
        //         } else {
        //             "text/plain"
        //         };
        //         // If it was a HEAD request, the server will only send the headers automatically
        //         HttpMockResponse::builder()
        //             .status(200)
        //             .header("content-type", content_type)
        //             .body(body)
        //             .build()
        //         // if matches!(req.method(), Method::GET) {
        //         //     let body = all_files2[&path].clone();
        //         //     let content_type = if path.ends_with(".json") {
        //         //         "application/json"
        //         //     } else {
        //         //         "text/plain"
        //         //     };
        //         //     response = response.header("content-type", content_type).body(body);
        //         // }
        //         // response.build()
        //     });
        // });
        // return mock;
    }
}
