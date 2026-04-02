use assert_cmd::Command;

#[test]
fn doctor_prints_capability_states() {
    let stdout = doctor_stdout();
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
