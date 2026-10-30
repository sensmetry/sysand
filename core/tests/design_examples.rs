// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The specification's examples and the tests that copy them cannot
//! drift apart.
//!
//! `core/src/manifest/mod_tests.rs` checks that every example manifest
//! in `model-toml.md` and `workspaces.md` parses, with the blocks copied
//! by hand. Each copy carries a marker naming the block it came from,
//! `// design: <document> #<n>` (counting fenced `toml` blocks in the
//! document), and this checks that the lines under each marker are the
//! lines of that block.
//!
//! Limits: it does not parse the blocks. It does not require a copy to
//! be byte-identical to its block (`model-toml.md` #1 is completed with
//! the required keys, and `workspaces.md` #4 is split in two, so two
//! copies may share a marker). It does not notice a line *deleted* from
//! a document.

const MODEL_TOML: &str = include_str!("../../design/model-toml.md");
const WORKSPACES_MD: &str = include_str!("../../design/workspaces.md");
const SYSIDE_METAMODEL_MD: &str = include_str!("../../design/syside-metamodel.md");
const COPIES: &str = include_str!("../src/manifest/mod_tests.rs");

/// The fenced `toml` blocks of a document, in order, each with the line
/// its fence opened on.
fn toml_blocks(document: &str) -> Vec<(usize, Vec<&str>)> {
    let mut blocks = Vec::new();
    let mut lines = document.lines().enumerate();
    while let Some((index, line)) = lines.next() {
        if line.trim() != "```toml" {
            continue;
        }
        let mut block = Vec::new();
        for (_, line) in lines.by_ref() {
            if line.trim() == "```" {
                break;
            }
            block.push(line);
        }
        blocks.push((index + 1, block));
    }
    blocks
}

/// The lines of a block that say something: a blank line carries no
/// content, and a comment-only line is prose. `# ...` in `model-toml.md`
/// #1 is the only comment-only line in any block; the `# invalid`
/// markers of `workspaces.md` #4 trail real assignments and are kept,
/// since dropping the document's own annotation would weaken the check
/// exactly where the document is making a point.
fn substantive(block: &[&str]) -> Vec<String> {
    block
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(collapse)
        .collect()
}

/// Whitespace runs to one space.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The copies' text, with Rust string syntax undone: `\"` is a quote,
/// and a backslash at the end of a line continues the literal, so the
/// line break and the indent that follows it are not content.
///
/// The `\n`s *inside* a literal are left as the two characters they are
/// written as. Turning them into line breaks and comparing whole lines
/// would fail on the four copies whose first line is glued to the
/// opening quote of the literal -- a difference in Rust syntax, not in
/// what the copy says.
fn as_written(source: &str) -> String {
    let joined: String = source
        .lines()
        .map(str::trim_end)
        .fold(String::new(), |mut acc, line| {
            if let Some(head) = acc.strip_suffix('\\') {
                acc = head.to_owned();
                acc.push_str(line.trim_start());
            } else {
                if !acc.is_empty() {
                    acc.push('\n');
                }
                acc.push_str(line);
            }
            acc
        });
    collapse(&joined.replace("\\\"", "\""))
}

/// Everything from a marker to the next one, which is the copy it
/// introduces.
fn under_marker(copies: &str, marker: &str) -> Vec<String> {
    let lines: Vec<&str> = copies.lines().collect();
    let mut slices = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if line.trim() != marker {
            continue;
        }
        let end = lines
            .iter()
            .skip(index + 1)
            .position(|line| line.trim().starts_with("// design:"))
            .map_or(lines.len(), |offset| index + 1 + offset);
        slices.push(lines[index + 1..end].join("\n"));
    }
    slices
}

#[test]
fn every_documented_example_is_copied_under_its_marker() {
    let mut missing = Vec::new();
    for (name, document) in [
        ("model-toml.md", MODEL_TOML),
        ("workspaces.md", WORKSPACES_MD),
        ("syside-metamodel.md", SYSIDE_METAMODEL_MD),
    ] {
        let blocks = toml_blocks(document);
        assert!(!blocks.is_empty(), "no fenced toml blocks in {name}");

        for (number, (line, block)) in blocks.iter().enumerate() {
            let marker = format!("// design: {name} #{}", number + 1);
            let slices = under_marker(COPIES, &marker);
            assert!(
                !slices.is_empty(),
                "{name}: the block at line {line} has no copy; \
                 add one to core/src/manifest/mod_tests.rs under `{marker}`"
            );
            let copied: Vec<String> = slices.iter().map(|slice| as_written(slice)).collect();
            for text in substantive(block) {
                if !copied.iter().any(|copy| copy.contains(&text)) {
                    missing.push(format!("{name} #{} (line {line}): {text}", number + 1));
                }
            }
        }

        // A marker naming a block the document no longer has: the copy
        // is then asserting something nothing documents.
        for number in blocks.len() + 1..blocks.len() + 4 {
            let marker = format!("// design: {name} #{number}");
            assert!(
                under_marker(COPIES, &marker).is_empty(),
                "{name}: a copy is marked `{marker}`, but the document has \
                 only {} fenced toml blocks",
                blocks.len()
            );
        }
    }

    assert!(
        missing.is_empty(),
        "the specification's examples have moved away from the tests that \
         copy them -- update core/src/manifest/mod_tests.rs:\n{}",
        missing.join("\n")
    );
}
