use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn version_prints_build_identity() {
    Command::cargo_bin("scaler")
        .unwrap()
        .arg("version")
        .assert()
        .success()
        .stdout(predicate::str::contains("scaler"))
        .stdout(predicate::str::contains(std::env::consts::OS))
        .stdout(predicate::str::contains(std::env::consts::ARCH));
}

#[test]
fn version_works_on_unsupported_hosts() {
    Command::cargo_bin("scaler")
        .unwrap()
        .arg("version")
        .assert()
        .success();
}
