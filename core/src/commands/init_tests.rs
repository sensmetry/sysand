// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::commands::init::{InitError, do_init_memory};
use crate::manifest::common::InheritFrom;
use crate::manifest::inherit::{Inherited, MetaProperty};

const OMG_SYSML: &str = "https://www.omg.org/spec/SysML/20250201";

fn init(metamodel: Option<MetaProperty<String>>) -> String {
    do_init_memory("widgets", "acme", "1.2.3", None, metamodel)
        .expect("the project is created")
        .model_toml
        .expect("a manifest")
}

/// model-toml.md "Metamodel": a project declares one, and `init` is
/// given it rather than choosing it.
#[test]
fn a_metamodel_is_written_when_given() {
    let written = init(Some(MetaProperty::Provided(OMG_SYSML.to_owned())));
    assert!(
        written.contains(&format!("\nmetamodel = \"{OMG_SYSML}\"\n")),
        "unexpected manifest: {written}"
    );
}

/// workspaces.md: "`metamodel.workspace = true` [...] satisfies its
/// requirement in `[project]`". What `init` writes inside a workspace
/// whose default group declares one.
#[test]
fn a_marker_is_written_when_asked_for() {
    let written = init(Some(MetaProperty::Inherited(Inherited {
        from: InheritFrom::DefaultGroup,
        at: None,
    })));
    assert!(
        written.contains("\nmetamodel.workspace = true\n"),
        "unexpected manifest: {written}"
    );
}

/// The value set is closed, so `init` cannot write one outside it.
#[test]
fn an_unrecognized_metamodel_is_refused() {
    let error = do_init_memory(
        "widgets",
        "acme",
        "1.2.3",
        None,
        Some(MetaProperty::Provided(
            "https://example.com/spec".to_owned(),
        )),
    )
    .expect_err("an unrecognized metamodel is refused");
    assert!(
        matches!(&error, InitError::Metamodel(value) if &**value == "https://example.com/spec"),
        "unexpected error: {error}"
    );
    assert!(
        error
            .to_string()
            .contains("https://www.sysand.com/spec/SysML/<date>"),
        "the message lists the four values: {error}"
    );
}

/// `init` takes the metamodel as an option: given none, it writes no
/// `metamodel` key.
#[test]
fn no_metamodel_is_written_when_none_is_given() {
    assert!(!init(None).contains("metamodel"));
}
