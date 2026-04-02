use assert_cmd::Command;

#[test]
fn doctor_prints_capability_states() {
    let stdout = doctor_stdout();
    if cfg!(target_os = "linux") {
        let lines = stdout.lines().collect::<Vec<_>>();

        assert!(lines.len() >= 6);
        assert_eq!(lines[0], "platform: linux");
        assert_eq!(lines[1], "backend: linux-systemd");
        assert!(lines[2].starts_with("backend_state: "));
        assert!(lines[3].starts_with("cpu: "));
        assert!(lines[4].starts_with("memory: "));
        assert!(lines[5].starts_with("interactive: "));
        assert!(
            lines[6..]
                .iter()
                .all(|line| line.starts_with("prerequisite: "))
        );
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
    assert!(
        stdout
            .lines()
            .filter(|line| line.starts_with("prerequisite: "))
            .all(|line| !line.trim().is_empty())
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
