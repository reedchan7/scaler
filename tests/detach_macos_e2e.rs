#![cfg(target_os = "macos")]

use std::ffi::OsString;
use std::thread::sleep;
use std::time::Duration;

use scaler::core::{LaunchPlan, Platform, ResourceSpec};
use scaler::detach::state::{RunState, StateRoot, read_result};

#[test]
#[ignore] // actually forks; run with: cargo test --test detach_macos_e2e -- --ignored
fn detach_launch_creates_run_and_finalizes() {
    let tmp = tempfile::tempdir().unwrap();
    // SAFETY: test is #[ignore] and opt-in; not run concurrently.
    unsafe {
        std::env::set_var("XDG_STATE_HOME", tmp.path());
    }

    let plan = LaunchPlan {
        argv: vec![
            OsString::from("sh"),
            OsString::from("-c"),
            OsString::from("echo hi"),
        ],
        resource_spec: ResourceSpec::default(),
        platform: Platform::Macos,
    };

    let id = scaler::detach::launch(&plan).expect("launch ok");

    let root = StateRoot::from_env().unwrap();
    let mut result = None;
    for _ in 0..50 {
        if let Ok(r) = read_result(&root, &id) {
            result = Some(r);
            break;
        }
        sleep(Duration::from_millis(100));
    }
    let r = result.expect("result.json was written within 5s");
    assert!(
        matches!(r.state, RunState::Exited),
        "expected Exited, got {:?}",
        r.state
    );
    assert_eq!(r.exit_code, Some(0));
}
