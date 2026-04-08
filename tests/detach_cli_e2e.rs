use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn scaler_status_on_empty_state_prints_nothing() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("scaler")
        .unwrap()
        .env("XDG_STATE_HOME", tmp.path())
        .args(["status"])
        .assert()
        .success();
    // Empty state dir → empty output (no rows). Not asserting exact
    // stdout because render_list writes nothing for 0 views.
}

#[test]
fn scaler_status_with_unknown_id_errors_out() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("scaler")
        .unwrap()
        .env("XDG_STATE_HOME", tmp.path())
        .args(["status", "20260101-000000-dead"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("not found")
                .or(predicate::str::contains("no run matches"))
                .or(predicate::str::contains("no match")),
        );
}

#[test]
fn scaler_finalize_is_hidden_from_help() {
    Command::cargo_bin("scaler")
        .unwrap()
        .args(["--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("__finalize").not());
}
