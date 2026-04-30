use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_lists_commands() {
    let mut command = Command::cargo_bin("sw-impact").expect("binary exists");

    command
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("index"))
        .stdout(predicate::str::contains("check"))
        .stdout(predicate::str::contains("query"));
}

#[test]
fn index_requires_plugin_path() {
    let mut command = Command::cargo_bin("sw-impact").expect("binary exists");

    command
        .args(["index", "--out", "index.sqlite"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--plugins"));
}
