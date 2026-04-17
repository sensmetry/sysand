use camino::{Utf8Path, Utf8PathBuf};
use chrono::DateTime;
use fluent_uri::Iri;
pub use httpmock;
use httpmock::{Method::GET, Method::HEAD, Mock, MockServer, Then, When};
use indexmap::{IndexMap, map::Entry};
use std::{collections::HashMap, fs, io::Write};
use thiserror::Error;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};
use urlencoding::encode;
use zip::{CompressionMethod, write::SimpleFileOptions};

use crate::{
    context::ProjectContext,
    include::do_include,
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw, InterchangeProjectUsageG},
    project::{ProjectMut, ProjectRead, memory::InMemoryProject, utils::ToUnixPathBuf},
    resolve::memory::{AcceptAll, MemoryResolver},
};

// pub type ProjectMock = InMemoryProject;

const INFO_PATH_STR: &str = ".project.json";
const METADATA_PATH_STR: &str = ".meta.json";

pub fn info_path() -> Utf8PathBuf {
    Utf8PathBuf::from(INFO_PATH_STR)
}
pub fn metadata_path() -> Utf8PathBuf {
    Utf8PathBuf::from(METADATA_PATH_STR)
}

// Use this instead of InMemoryProject to allow malformed .project.json and .meta.json
// The path type is interpreted as the current OS would interpret it
// So on Windows, the path might use Windows path separators
#[derive(Clone)]
pub struct ProjectMock {
    pub all_files: HashMap<Utf8PathBuf, String>,
}

pub struct ProjectMockBuilder {
    pub in_memory_project: InMemoryProject,
}

fn into<T>(option: Option<impl Into<T>>) -> Option<T> {
    option.map(|value| value.into())
}

#[derive(Clone, Debug)]
pub enum Created {
    Custom(String),
    Minimum,
    Now,
}

impl ProjectMockBuilder {
    pub fn new(name: impl Into<String>, version: impl Into<String>, created: Created) -> Self {
        Self {
            in_memory_project: InMemoryProject::from_info_meta(
                InterchangeProjectInfoRaw::minimal(name.into(), version.into()),
                InterchangeProjectMetadataRaw {
                    index: IndexMap::default(),
                    created: match created {
                        Created::Custom(time) => time,
                        Created::Minimum => DateTime::<chrono::Utc>::MIN_UTC.to_rfc3339(),
                        Created::Now => chrono::Utc::now().to_rfc3339(),
                    },
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

    // pub fn with_created(self: &mut Self, created: impl Into<String>) -> &mut Self {
    //     self.meta_mut().created = created.into();
    //     self
    // }

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
                    &INFO_PATH_STR.into(),
                    &serde_json::to_string(&self.info()).unwrap(),
                ),
                (
                    &METADATA_PATH_STR.into(),
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

#[derive(Clone, Copy, Debug)]
pub enum ZipOptions {
    Custom(SimpleFileOptions),
    Default,
}

pub struct Mocks<'a> {
    pub head: HashMap<Utf8PathBuf, Mock<'a>>,
    pub get: HashMap<Utf8PathBuf, Mock<'a>>,
}

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

    pub fn builder(
        name: impl Into<String>,
        version: impl Into<String>,
        created: Created,
    ) -> ProjectMockBuilder {
        ProjectMockBuilder::new(name, version, created)
    }

    pub fn new_small_example() -> Self {
        Self::builder("Lib test", "0.0.1", Created::Minimum)
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

    pub fn save_to_folder(self: &Self, root_path: &Utf8Path) {
        for (path, contents) in self.all_files.iter() {
            let full_path: Utf8PathBuf = [root_path, path].iter().collect();
            fs::write(full_path, contents).unwrap();
        }
    }

    pub fn to_zip(self: &Self, options: ZipOptions) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        self.to_zip_internal(None, options)
    }

    pub fn to_zip_non_standard(
        self: &Self,
        base_path: &str,
        options: ZipOptions,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        self.to_zip_internal(Some(base_path), options)
    }

    fn to_zip_internal(
        self: &Self,
        base_path: Option<&str>,
        options: ZipOptions,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut cursor = std::io::Cursor::new(vec![]);
        let mut zip = zip::ZipWriter::new(&mut cursor);

        let options = match options {
            ZipOptions::Custom(options) => options,
            ZipOptions::Default => SimpleFileOptions::default()
                .compression_method(CompressionMethod::Stored)
                .unix_permissions(0o755),
        };

        for (file_path, file_contents) in self.all_files.iter() {
            let path = match base_path {
                Some(base_path) => Utf8PathBuf::from(base_path).join(file_path),
                None => file_path.clone(),
            };
            let path = path.to_unix_path_buf();
            zip.start_file(path, options)?;
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

    pub fn add_zip_to_server() {}

    pub fn add_files_to_server<'a>(
        &self,
        server: &'a MockServer,
        mut when_fn: impl FnMut(When) -> When,
        mut then_fn: impl FnMut(Then) -> Then,
    ) -> Mocks<'a> {
        let mut mocks = Mocks {
            head: HashMap::new(),
            get: HashMap::new(),
        };
        for (path, content) in self.all_files.iter() {
            let content_type = if path.ends_with(".json") {
                "application/json"
            } else {
                "text/plain"
            };
            let mut when_fn = |when| when_fn(when).path(Self::path_to_url_path(path));
            let mut then_fn = |then| then_fn(then).header("content-type", content_type);
            let head_mock = server.mock(|when, then| {
                when_fn(when).method(HEAD);
                then_fn(then).status(200);
            });
            let get_mock = server.mock(|when, then| {
                when_fn(when).method(GET);
                then_fn(then).status(200).body(content);
            });
            mocks.head.insert(path.clone(), head_mock);
            mocks.get.insert(path.clone(), get_mock);
            // (path.into(), mock)
        }
        mocks
        // self.all_files
        //     .iter()
        //     .map(|(path, content)| {
        //         // for (path, content) in self.all_files.iter() {
        //         let mock = server.mock(|when, then| {
        //             // println!("{}", Self::path_to_url_path(path));
        //             when_fn(when)
        //                 .method(GET)
        //                 // .method(HEAD)
        //                 .path(Self::path_to_url_path(path));
        //             let content_type = if path.ends_with(".json") {
        //                 "application/json"
        //             } else {
        //                 "text/plain"
        //             };
        //             then_fn(then)
        //                 .status(200)
        //                 .header("content-type", content_type)
        //                 .body(content);
        //         });
        //         (path.into(), mock)
        //     })
        //     .collect()
    }

    // pub fn add_to_server<'a>(
    //     &self,
    //     server: &'a MockServer,
    //     mut when_fn: impl FnMut(When) -> When,
    //     mut then_fn: impl FnMut(Then) -> Then,
    // ) -> HashMap<Utf8PathBuf, Mock<'a>> {
    //     self.all_files
    //         .iter()
    //         .map(|(path, content)| {
    //             // for (path, content) in self.all_files.iter() {
    //             let mock = server.mock(|when, then| {
    //                 // println!("{}", Self::path_to_url_path(path));
    //                 when_fn(when)
    //                     .method(GET)
    //                     // .method(HEAD)
    //                     .path(Self::path_to_url_path(path));
    //                 let content_type = if path.ends_with(".json") {
    //                     "application/json"
    //                 } else {
    //                     "text/plain"
    //                 };
    //                 then_fn(then)
    //                     .status(200)
    //                     .header("content-type", content_type)
    //                     .body(content);
    //             });
    //             (path.into(), mock)
    //         })
    //         .collect()
    // }
}

pub fn mock_resolver<'a, I: IntoIterator<Item = (&'a str, ProjectMock)>>(
    projects: I,
) -> MemoryResolver<AcceptAll, ProjectMock> {
    MemoryResolver {
        iri_predicate: AcceptAll {},
        projects: HashMap::from_iter(
            projects
                .into_iter()
                .map(|(k, v)| (Iri::parse(k.to_string()).unwrap(), vec![v])),
        ),
    }
}

#[derive(Error, Debug)]
pub enum ProjectMockError {
    #[error(".project.json is malformed: {0}")]
    InfoMalformed(serde_json::Error),
    #[error(".meta.json is malformed: {0}")]
    MetaMalformed(serde_json::Error),
    // #[error("{0}")]
    // AlreadyExists(String),
    #[error("project read error: file `{0}` not found")]
    FileNotFound(Utf8PathBuf),
    // #[error("failed to read from reader: {0}")]
    // IoRead(#[from] std::io::Error),
}

impl ProjectRead for ProjectMock {
    type Error = ProjectMockError;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        ProjectMockError,
    > {
        let info = self
            .all_files
            .get(&info_path())
            .map(|info| serde_json::from_str(info))
            .transpose();
        let meta = self
            .all_files
            .get(&metadata_path())
            .map(|meta| serde_json::from_str(meta))
            .transpose();
        match (info, meta) {
            (Ok(info), Ok(meta)) => Ok((info, meta)),
            (Err(info_err), _) => Err(ProjectMockError::InfoMalformed(info_err)),
            (_, Err(meta_err)) => Err(ProjectMockError::MetaMalformed(meta_err)),
        }
    }

    type SourceReader<'a> = &'a [u8];

    fn read_source<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, ProjectMockError> {
        let path_buf = Utf8PathBuf::from(path.as_ref().as_str());
        let contents = self
            .all_files
            .get(&path_buf)
            .ok_or(ProjectMockError::FileNotFound(path_buf))?;

        Ok(contents.as_bytes())
    }

    fn sources(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, ProjectMockError> {
        panic!("No sources for the ProjectMock are known")
    }
}
