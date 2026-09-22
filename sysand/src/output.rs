// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! `--format json`: one JSON document on stdout per run, warnings and
//! progress on stderr as in text mode.
//!
//! Every document carries the envelope `schemaVersion` (1), `complete`,
//! `skipped` and `escapeHatch` -- the partiality fields, carried on
//! every Sysand document so that no reader has to know which documents
//! can be partial -- and camelCase keys.
//! Within a major, fields are added, never removed or retyped.
//!
//! A command whose output is a document (`info`, `sources`, `env list`,
//! `print-root`, `versions`, `lock`, `sync`) fills the payload through
//! [`document`] / [`push`]; a mutating command prints a *result document*
//! (`command`, `changed`); a failing run prints a *failure document*
//! (`complete: false`, `error: {kind, message}`) and keeps its exit
//! code. `--format json` is refused only where a document has no
//! meaning.
//!
//! The state is per run and reset by [`begin`]: the embedded CLI runs
//! many times in one process.

use std::sync::{LazyLock, Mutex};

use camino::Utf8Path;
use serde_json::{Map, Value, json};

use crate::cli::OutputFormat;

/// The documents' `schemaVersion`.
pub const SCHEMA_VERSION: u64 = 1;

/// The refusal of `--format json` by a command that has no document.
pub fn not_supported(command: &str) -> String {
    format!("`--format json` is not supported by `{command}`")
}

struct State {
    format: OutputFormat,
    payload: Map<String, Value>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| {
    Mutex::new(State {
        format: OutputFormat::Text,
        payload: Map::new(),
    })
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Start a run in `format`, discarding anything a previous run left.
pub fn begin(format: OutputFormat) {
    let mut state = state();
    state.format = format;
    state.payload = Map::new();
}

pub fn is_json() -> bool {
    matches!(state().format, OutputFormat::Json)
}

/// Run `print` only in text mode.
pub fn text(print: impl FnOnce()) {
    if !is_json() {
        print();
    }
}

/// Set one field of the document's payload (json mode; a no-op in text
/// mode, so a site can always call it beside its text printing).
pub fn document(key: &str, value: impl Into<Value>) {
    let mut state = state();
    if matches!(state.format, OutputFormat::Json) {
        state.payload.insert(key.to_owned(), value.into());
    }
}

/// Append `item` to the array at `key` (created on first use).
pub fn push(key: &str, item: impl Into<Value>) {
    let mut state = state();
    if !matches!(state.format, OutputFormat::Json) {
        return;
    }
    match state
        .payload
        .entry(key.to_owned())
        .or_insert_with(|| Value::Array(vec![]))
    {
        Value::Array(items) => items.push(item.into()),
        other => *other = Value::Array(vec![item.into()]),
    }
}

/// One project's values for a `get` verb of `info`: `member` names the
/// workspace member (the text form's prefix without its `: `), `None`
/// for a single project.
pub fn values(member: Option<&str>, values: &[String]) {
    push("values", json!({ "member": member, "values": values }));
}

fn envelope(complete: bool, payload: Map<String, Value>) -> Value {
    let mut document = Map::new();
    document.insert("schemaVersion".to_owned(), json!(SCHEMA_VERSION));
    document.insert("complete".to_owned(), json!(complete));
    document.insert("skipped".to_owned(), json!([]));
    document.insert("escapeHatch".to_owned(), Value::Null);
    document.extend(payload);
    Value::Object(document)
}

fn print(document: &Value) {
    let text = serde_json::to_string_pretty(document).expect("a JSON value serializes");
    println!("{text}");
}

/// What a run changed, for the result document of a mutating command.
#[derive(Debug, Clone, Default)]
pub struct RunReport {
    /// The command's name as the user spelled it (`"env install"`).
    pub command: String,
    /// The paths the command writes, by kind -- not observed on disk.
    pub changed: Vec<String>,
    /// Extra result fields (`published`, ...).
    pub extra: Map<String, Value>,
}

impl RunReport {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn changed(mut self, path: impl AsRef<Utf8Path>) -> Self {
        self.changed.push(path.as_ref().to_string());
        self
    }

    #[must_use]
    pub fn changed_all<P: AsRef<Utf8Path>>(mut self, paths: impl IntoIterator<Item = P>) -> Self {
        for path in paths {
            self.changed.push(path.as_ref().to_string());
        }
        self
    }

    #[must_use]
    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.extra.insert(key.to_owned(), value.into());
        self
    }
}

/// The run succeeded: print the document the command filled, or the
/// result document of `report` when it filled none. Text mode prints
/// nothing here.
pub fn finish_ok(report: &RunReport) {
    let mut state = state();
    if !matches!(state.format, OutputFormat::Json) {
        return;
    }
    let mut payload = std::mem::take(&mut state.payload);
    drop(state);
    // Dependency sources come out of the solver in hash order; the
    // document's list is sorted so two runs agree (the text form keeps
    // the manifest's own-then-dependencies order).
    if let Some(Value::Array(sources)) = payload.get_mut("sources") {
        sources.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    }
    let payload = if payload.is_empty() {
        let mut result = Map::new();
        result.insert("command".to_owned(), json!(report.command));
        result.insert("changed".to_owned(), json!(report.changed));
        result.extend(report.extra.clone());
        result
    } else {
        payload
    };
    print(&envelope(true, payload));
}

/// The run failed: print the failure document. Text mode prints nothing
/// here (the caller prints the error chain to stderr either way).
pub fn finish_err(kind: &str, message: &str) {
    let mut state = state();
    if !matches!(state.format, OutputFormat::Json) {
        return;
    }
    let mut payload = std::mem::take(&mut state.payload);
    drop(state);
    payload.insert(
        "error".to_owned(),
        json!({ "kind": kind, "message": message }),
    );
    print(&envelope(false, payload));
}

/// The `lock` document: the lockfile's path and its projects in the
/// canonical (sorted) order, sources as their inline-table text.
pub fn lock_document(lock: &sysand_core::lock::Lock, lockfile: &Utf8Path) {
    if !is_json() {
        return;
    }
    let projects: Vec<Value> = lock
        .projects
        .iter()
        .map(|project| {
            json!({
                "publisher": project.publisher,
                "name": project.name,
                "version": project.version,
                "metamodel": project.metamodel,
                "exports": project.exports,
                "identifiers": project.identifiers,
                "usages": project.usages.iter().map(sysand_core::lock::Usage::inner).collect::<Vec<_>>(),
                "sources": project.sources.iter().map(|source| source.to_toml().to_string()).collect::<Vec<_>>(),
            })
        })
        .collect();
    document("lockfile", lockfile.as_str());
    document("projects", projects);
}

fn synced(projects: &[sysand_core::commands::sync::SyncedProject]) -> Value {
    json!(
        projects
            .iter()
            .map(|project| json!({ "iri": project.iri, "version": project.version }))
            .collect::<Vec<_>>()
    )
}

/// The `sync` document: the outcome's lists (`SyncOutcome`), every
/// finding with the text the CLI prints.
pub fn sync_document(outcome: &sysand_core::commands::sync::SyncOutcome) {
    use sysand_core::commands::sync::MismatchKind;
    if !is_json() {
        return;
    }
    document("installed", synced(&outcome.installed));
    document("pruned", synced(&outcome.pruned));
    document("kept", synced(&outcome.kept));
    document("verified", synced(&outcome.verified));
    document("repaired", synced(&outcome.repaired));
    document(
        "mismatched",
        outcome
            .mismatched
            .iter()
            .map(|mismatch| {
                json!({
                    "iri": mismatch.iri,
                    "version": mismatch.version,
                    "kind": mismatch.kind.name(),
                    "file": match &mismatch.kind {
                        MismatchKind::Checksum { file: Some(file) } => Some(file.as_str()),
                        _ => None,
                    },
                    "message": mismatch.to_string(),
                })
            })
            .collect::<Vec<_>>(),
    );
    document(
        "unrecorded",
        outcome
            .unrecorded
            .iter()
            .map(|path| path.as_str())
            .collect::<Vec<_>>(),
    );
    document(
        "stale",
        outcome
            .stale
            .iter()
            .map(|stale| {
                json!({
                    "manifest": stale.manifest.as_str(),
                    "usage": stale.usage,
                    "kind": stale.kind.name(),
                    "message": stale.to_string(),
                })
            })
            .collect::<Vec<_>>(),
    );
}
