use assert_cmd::Command;

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
        assert_eq!(lines[6], linux_prerequisite_line("cgroup_v2", &stdout));
        assert_eq!(lines[7], linux_prerequisite_line("user_manager", &stdout));
        assert_sorted_warning_lines(&lines[8..]);
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
        assert_eq!(lines[6], macos_prerequisite_line("taskpolicy", &stdout));
        assert_eq!(
            lines[7],
            macos_prerequisite_line("platform_version", &stdout)
        );
        assert_sorted_warning_lines(&lines[8..]);
    } else {
        let expected = concat!(
            "platform: unsupported\n",
            "backend: unsupported\n",
            "backend_state: unavailable\n",
            "cpu: unavailable\n",
            "memory: unavailable\n",
            "interactive: unavailable\n",
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

fn linux_prerequisite_line(key: &str, stdout: &str) -> String {
    let status = match key {
        "cgroup_v2" if stdout.contains("warning: unified cgroup v2 is not available") => "missing",
        "user_manager" if stdout.contains("warning: systemd user manager is unreachable") => {
            "unreachable"
        }
        "cgroup_v2" | "user_manager" => "ok",
        _ => unreachable!(),
    };

    format!("prerequisite: {key}={status}")
}

fn macos_prerequisite_line(key: &str, stdout: &str) -> String {
    let status = match key {
        "taskpolicy" if stdout.contains("warning: taskpolicy is not available in PATH") => {
            "missing"
        }
        "platform_version"
            if stdout.contains(
                "warning: macOS platform version is not supported by the taskpolicy backend",
            ) =>
        {
            "unsupported"
        }
        "taskpolicy" | "platform_version" => "ok",
        _ => unreachable!(),
    };

    format!("prerequisite: {key}={status}")
}
