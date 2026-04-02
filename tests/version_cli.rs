use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn version_prints_build_identity() {
    let os_arch = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);

    Command::cargo_bin("scaler")
        .unwrap()
        .arg("version")
        .assert()
        .success()
        .stdout(predicate::str::contains("scaler"))
        .stdout(predicate::str::contains(&os_arch));
}

#[test]
fn version_works_on_unsupported_hosts() {
    Command::cargo_bin("scaler")
        .unwrap()
        .arg("version")
        .assert()
        .success();
}
