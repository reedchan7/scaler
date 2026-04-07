use assert_cmd::Command;
use scaler::{
    cli::render_doctor_output,
    core::{
        BackendKind, CapabilityLevel, CapabilityReport, DoctorPrerequisite, Platform,
        PrerequisiteStatus,
    },
};

#[test]
fn doctor_prints_capability_states() {
    let stdout = doctor_stdout();
    let lines = stdout.lines().collect::<Vec<_>>();

    if cfg!(target_os = "linux") {
        assert_core_lines(
            &lines,
            &[
                "platform: linux",
                "backend: linux_systemd",
                "backend_state: ",
                "cpu: ",
                "memory: ",
                "interactive: ",
            ],
        );
        assert!(lines[6].starts_with("effective_backend: "));
        assert_line_prefixes(
            &lines[7..10],
            &[
                "prerequisite: systemd_run=",
                "prerequisite: cgroup_v2=",
                "prerequisite: user_manager=",
            ],
        );
        assert_sorted_warning_lines(&lines[10..]);
    } else if cfg!(target_os = "macos") {
        assert_core_lines(
            &lines,
            &[
                "platform: macos",
                "backend: macos_taskpolicy",
                "backend_state: ",
                "cpu: ",
                "memory: ",
                "interactive: ",
            ],
        );
        assert!(lines[6].starts_with("effective_backend: "));
        assert_line_prefixes(
            &lines[7..9],
            &[
                "prerequisite: taskpolicy=",
                "prerequisite: platform_version=",
            ],
        );
        assert_sorted_warning_lines(&lines[9..]);
    } else {
        let expected = concat!(
            "platform: unsupported\n",
            "backend: unsupported\n",
            "backend_state: unavailable\n",
            "cpu: unavailable\n",
            "memory: unavailable\n",
            "interactive: unavailable\n",
            "effective_backend: plain_fallback\n",
            "prerequisite: no supported backend for this host\n",
        );

        assert_eq!(stdout, expected);
    }
}

#[test]
fn doctor_uses_only_known_capability_words() {
    let stdout = doctor_stdout();
    let known = ["enforced", "best_effort", "unavailable"];
    let capability_values = stdout
        .lines()
        .filter_map(|line| line.split_once(": "))
        .filter_map(|(field, value)| match field {
            "backend_state" | "cpu" | "memory" | "interactive" => Some(value),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(capability_values.len(), 4);
    assert!(capability_values.iter().all(|value| known.contains(value)));
    assert!(stdout.lines().all(|line| !line.trim().is_empty()));
    assert!(
        stdout
            .lines()
            .any(|line| line.starts_with("prerequisite: "))
    );
    assert!(
        stdout
            .lines()
            .filter(|line| line.starts_with("warning: "))
            .collect::<Vec<_>>()
            .windows(2)
            .all(|pair| pair[0] <= pair[1])
    );
}

#[test]
fn doctor_renderer_uses_structured_prerequisites_in_declared_order() {
    let report = CapabilityReport {
        platform: Platform::Linux,
        backend: BackendKind::LinuxSystemd,
        backend_state: CapabilityLevel::Unavailable,
        cpu: CapabilityLevel::Unavailable,
        memory: CapabilityLevel::Unavailable,
        interactive: CapabilityLevel::Unavailable,
        prerequisites: vec![
            DoctorPrerequisite::check("systemd_run", PrerequisiteStatus::Missing),
            DoctorPrerequisite::check("cgroup_v2", PrerequisiteStatus::Missing),
            DoctorPrerequisite::check("user_manager", PrerequisiteStatus::Skipped),
        ],
        warnings: vec!["z warning".to_string(), "a warning".to_string()],
    };

    let stdout = render_doctor_output(&report, BackendKind::PlainFallback);
    let expected = concat!(
        "platform: linux\n",
        "backend: linux_systemd\n",
        "backend_state: unavailable\n",
        "cpu: unavailable\n",
        "memory: unavailable\n",
        "interactive: unavailable\n",
        "effective_backend: plain_fallback\n",
        "prerequisite: systemd_run=missing\n",
        "prerequisite: cgroup_v2=missing\n",
        "prerequisite: user_manager=skipped\n",
        "warning: a warning\n",
        "warning: z warning",
    );

    assert_eq!(stdout, expected);
}

fn doctor_stdout() -> String {
    let output = Command::cargo_bin("scaler")
        .unwrap()
        .arg("doctor")
        .output()
        .unwrap();

    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}

fn assert_core_lines(lines: &[&str], expected: &[&str; 6]) {
    assert!(lines.len() >= 8);
    assert_eq!(lines[0], expected[0]);
    assert_eq!(lines[1], expected[1]);
    assert!(lines[2].starts_with(expected[2]));
    assert!(lines[3].starts_with(expected[3]));
    assert!(lines[4].starts_with(expected[4]));
    assert!(lines[5].starts_with(expected[5]));
}

fn assert_sorted_warning_lines(lines: &[&str]) {
    assert!(lines.iter().all(|line| line.starts_with("warning: ")));
    assert!(lines.windows(2).all(|pair| pair[0] <= pair[1]));
}

#[test]
fn doctor_smoke_output_includes_platform_backend_state_and_prerequisite() {
    let stdout = doctor_stdout();

    assert!(stdout.lines().any(|line| line.starts_with("platform: ")));
    assert!(
        stdout
            .lines()
            .any(|line| line.starts_with("backend_state: "))
    );
    assert!(
        stdout
            .lines()
            .any(|line| line.starts_with("prerequisite: "))
    );
}

fn assert_line_prefixes(lines: &[&str], expected_prefixes: &[&str]) {
    assert_eq!(lines.len(), expected_prefixes.len());
    for (line, prefix) in lines.iter().zip(expected_prefixes.iter()) {
        assert!(
            line.starts_with(prefix),
            "{line} did not start with {prefix}"
        );
    }
}

#[test]
fn doctor_renderer_emits_effective_backend_line() {
    let report = CapabilityReport {
        platform: Platform::Linux,
        backend: BackendKind::LinuxSystemd,
        backend_state: CapabilityLevel::Enforced,
        cpu: CapabilityLevel::Enforced,
        memory: CapabilityLevel::Enforced,
        interactive: CapabilityLevel::Enforced,
        prerequisites: vec![
            DoctorPrerequisite::check("systemd_run", PrerequisiteStatus::Ok),
            DoctorPrerequisite::check("cgroup_v2", PrerequisiteStatus::Ok),
            DoctorPrerequisite::check("user_manager", PrerequisiteStatus::Ok),
        ],
        warnings: vec![],
    };

    let stdout = render_doctor_output(&report, BackendKind::LinuxSystemd);

    assert!(
        stdout.contains("effective_backend: linux_systemd"),
        "expected effective_backend line, got: {stdout}"
    );
}
