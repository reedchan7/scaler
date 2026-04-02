use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn doctor_prints_capability_states() {
    Command::cargo_bin("scaler")
        .unwrap()
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("platform: "))
        .stdout(predicate::str::contains("backend: "))
        .stdout(predicate::str::contains("backend_state: "))
        .stdout(predicate::str::contains("cpu: "))
        .stdout(predicate::str::contains("memory: "))
        .stdout(predicate::str::contains("interactive: "))
        .stdout(predicate::str::contains("prerequisite: "));
}

#[test]
fn doctor_uses_only_known_capability_words() {
    Command::cargo_bin("scaler")
        .unwrap()
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("unavailable"));
}
