// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use super::highest_first;

fn sorted(versions: &[Result<&str, &str>]) -> Vec<Result<String, String>> {
    highest_first(
        versions
            .iter()
            .map(|v| v.map(ToOwned::to_owned).map_err(ToOwned::to_owned)),
    )
}

fn ok(versions: &[&str]) -> Vec<Result<String, String>> {
    versions.iter().map(|v| Ok((*v).to_owned())).collect()
}

#[test]
fn sorts_by_semver_precedence_not_as_strings() {
    assert_eq!(
        sorted(&[Ok("1.9.0"), Ok("1.10.0"), Ok("1.0.0"), Ok("1.0.0-alpha.1")]),
        ok(&["1.10.0", "1.9.0", "1.0.0", "1.0.0-alpha.1"])
    );
}

#[test]
fn keeps_an_ordered_list_as_it_is() {
    let descending = ["2.0.0", "1.2.0", "1.2.0-rc.1", "0.1.0"];
    assert_eq!(sorted(&descending.map(Ok)), ok(&descending));
}

#[test]
fn keeps_the_order_of_equal_precedence() {
    assert_eq!(
        sorted(&[Ok("1.0.0+b"), Ok("1.0.0+a"), Ok("0.9.0")]),
        ok(&["1.0.0+b", "1.0.0+a", "0.9.0"])
    );
}

#[test]
fn puts_what_is_not_semver_last_in_its_order() {
    assert_eq!(
        sorted(&[
            Ok("latest"),
            Err("broken"),
            Ok("1.0.0"),
            Ok("1"),
            Ok("2.0.0")
        ]),
        vec![
            Ok("2.0.0".to_owned()),
            Ok("1.0.0".to_owned()),
            Ok("latest".to_owned()),
            Err("broken".to_owned()),
            Ok("1".to_owned()),
        ]
    );
}
