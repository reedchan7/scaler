#[cfg(target_os = "linux")]
mod linux_tests {
    use std::ffi::OsString;

    use scaler::{
        backend::linux_systemd::{LinuxProbe, build_systemd_run_argv, detect_linux_capabilities},
        core::{
            BackendKind, CapabilityLevel, CpuLimit, InteractiveMode, LaunchPlan, MemoryLimit,
            Platform, PrerequisiteStatus, ResourceSpec, ShellKind,
        },
    };

    #[test]
    fn linux_command_uses_scope_and_memory_mapping() {
        let plan = LaunchPlan {
            argv: vec![OsString::from("echo"), OsString::from("ok")],
            resource_spec: ResourceSpec {
                cpu: Some(CpuLimit::from_centi_cores(100)),
                mem: Some(MemoryLimit::from_bytes(1_073_741_824)),
                interactive: InteractiveMode::Never,
                shell: None,
                monitor: true,
            },
            platform: Platform::Linux,
        };

        let argv = build_systemd_run_argv(&plan).unwrap();
        let argv = argv
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(argv[0], "systemd-run");
        assert!(argv.iter().any(|value| value == "--user"));
        assert!(argv.iter().any(|value| value == "--scope"));
        assert!(argv.iter().any(|value| value == "--property=CPUQuota=100%"));
        assert!(
            argv.iter()
                .any(|value| value == "--property=MemoryHigh=966367642")
        );
        assert!(
            argv.iter()
                .any(|value| value == "--property=MemoryMax=1073741824")
        );
        assert!(
            argv.iter()
                .any(|value| value == "--property=MemorySwapMax=0")
        );
        let delimiter_index = argv.iter().position(|value| value == "--").unwrap();
        assert_eq!(delimiter_index, 7);
        assert_eq!(&argv[delimiter_index + 1..], ["echo", "ok"]);
    }

    #[test]
    fn linux_command_preserves_dash_prefixed_executable_after_delimiter() {
        let plan = LaunchPlan {
            argv: vec![OsString::from("-tool"), OsString::from("arg")],
            resource_spec: ResourceSpec::default(),
            platform: Platform::Linux,
        };

        let argv = build_systemd_run_argv(&plan).unwrap();
        let argv = argv
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            argv,
            vec![
                "systemd-run".to_string(),
                "--user".to_string(),
                "--scope".to_string(),
                "--".to_string(),
                "-tool".to_string(),
                "arg".to_string(),
            ]
        );
    }

    #[test]
    fn linux_command_wraps_shell_script_when_requested() {
        let plan = LaunchPlan {
            argv: vec![OsString::from("echo ok")],
            resource_spec: ResourceSpec {
                shell: Some(ShellKind::Sh),
                ..ResourceSpec::default()
            },
            platform: Platform::Linux,
        };

        let argv = build_systemd_run_argv(&plan).unwrap();
        let argv = argv
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            argv,
            vec![
                "systemd-run".to_string(),
                "--user".to_string(),
                "--scope".to_string(),
                "--".to_string(),
                "sh".to_string(),
                "-lc".to_string(),
                "echo ok".to_string(),
            ]
        );
    }

    #[test]
    fn linux_command_rejects_multiple_shell_tokens() {
        let plan = LaunchPlan {
            argv: vec![OsString::from("echo"), OsString::from("ok")],
            resource_spec: ResourceSpec {
                shell: Some(ShellKind::Sh),
                ..ResourceSpec::default()
            },
            platform: Platform::Linux,
        };

        let error = build_systemd_run_argv(&plan).unwrap_err().to_string();

        assert!(error.contains("exactly one script token"));
    }

    #[test]
    fn linux_detect_reports_missing_systemd_run() {
        let report = detect_linux_capabilities(LinuxProbe {
            has_systemd_run: false,
            has_cgroup_v2: true,
            user_manager_reachable: true,
        });

        assert_eq!(report.platform, Platform::Linux);
        assert_eq!(report.backend, BackendKind::LinuxSystemd);
        assert_eq!(report.backend_state, CapabilityLevel::Unavailable);
        assert_eq!(report.cpu, CapabilityLevel::Unavailable);
        assert_eq!(report.memory, CapabilityLevel::Unavailable);
        assert_eq!(report.interactive, CapabilityLevel::Unavailable);
        assert_prerequisite(
            &report.prerequisites[0],
            "systemd_run",
            PrerequisiteStatus::Missing,
        );
        assert_prerequisite(
            &report.prerequisites[1],
            "cgroup_v2",
            PrerequisiteStatus::Ok,
        );
        assert_prerequisite(
            &report.prerequisites[2],
            "user_manager",
            PrerequisiteStatus::Skipped,
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("systemd-run"))
        );
        assert!(
            !report
                .warnings
                .iter()
                .any(|warning| warning.contains("user manager"))
        );
    }

    #[test]
    fn linux_detect_reports_enforced_happy_path() {
        let report = detect_linux_capabilities(LinuxProbe {
            has_systemd_run: true,
            has_cgroup_v2: true,
            user_manager_reachable: true,
        });

        assert_eq!(report.backend_state, CapabilityLevel::Enforced);
        assert_eq!(report.cpu, CapabilityLevel::Enforced);
        assert_eq!(report.memory, CapabilityLevel::Enforced);
        assert_eq!(report.interactive, CapabilityLevel::Enforced);
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn linux_detect_reports_missing_cgroup_v2() {
        let report = detect_linux_capabilities(LinuxProbe {
            has_systemd_run: true,
            has_cgroup_v2: false,
            user_manager_reachable: true,
        });

        assert_eq!(report.backend_state, CapabilityLevel::Unavailable);
        assert_eq!(report.cpu, CapabilityLevel::Unavailable);
        assert_eq!(report.memory, CapabilityLevel::Unavailable);
        assert_eq!(report.interactive, CapabilityLevel::Unavailable);
        assert_prerequisite(
            &report.prerequisites[0],
            "systemd_run",
            PrerequisiteStatus::Ok,
        );
        assert_prerequisite(
            &report.prerequisites[1],
            "cgroup_v2",
            PrerequisiteStatus::Missing,
        );
        assert_prerequisite(
            &report.prerequisites[2],
            "user_manager",
            PrerequisiteStatus::Ok,
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("cgroup"))
        );
    }

    #[test]
    fn linux_detect_reports_missing_user_manager() {
        let report = detect_linux_capabilities(LinuxProbe {
            has_systemd_run: true,
            has_cgroup_v2: true,
            user_manager_reachable: false,
        });

        assert_eq!(report.backend_state, CapabilityLevel::Unavailable);
        assert_eq!(report.cpu, CapabilityLevel::Unavailable);
        assert_eq!(report.memory, CapabilityLevel::Unavailable);
        assert_eq!(report.interactive, CapabilityLevel::Unavailable);
        assert_prerequisite(
            &report.prerequisites[0],
            "systemd_run",
            PrerequisiteStatus::Ok,
        );
        assert_prerequisite(
            &report.prerequisites[1],
            "cgroup_v2",
            PrerequisiteStatus::Ok,
        );
        assert_prerequisite(
            &report.prerequisites[2],
            "user_manager",
            PrerequisiteStatus::Unreachable,
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("user manager"))
        );
    }

    #[test]
    fn linux_backend_command_preview_includes_systemd_run_and_resource_properties() {
        let plan = LaunchPlan {
            argv: vec![OsString::from("/bin/echo"), OsString::from("hi")],
            resource_spec: ResourceSpec {
                cpu: Some(CpuLimit::from_centi_cores(50)),
                mem: Some(MemoryLimit::from_bytes(67_108_864)),
                interactive: InteractiveMode::Never,
                shell: None,
                monitor: false,
            },
            platform: Platform::Linux,
        };

        let preview =
            scaler::backend::linux_systemd::linux_systemd_command_preview_for_test(&plan).unwrap();
        let preview = preview
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(preview[0], "systemd-run");
        assert!(preview.iter().any(|value| value == "--user"));
        assert!(preview.iter().any(|value| value == "--scope"));
        assert!(
            preview
                .iter()
                .any(|value| value == "--property=CPUQuota=50%")
        );
        assert!(
            preview
                .iter()
                .any(|value| value == "--property=MemoryMax=67108864")
        );
        assert!(
            preview
                .iter()
                .any(|value| value == "--property=MemorySwapMax=0")
        );
        let dash_dash = preview.iter().position(|value| value == "--").unwrap();
        assert_eq!(&preview[dash_dash + 1..], &["/bin/echo", "hi"]);
    }

    fn assert_prerequisite(
        prerequisite: &scaler::core::DoctorPrerequisite,
        expected_key: &'static str,
        expected_status: PrerequisiteStatus,
    ) {
        match prerequisite {
            scaler::core::DoctorPrerequisite::Check { key, status } => {
                assert_eq!(*key, expected_key);
                assert_eq!(*status, expected_status);
            }
            scaler::core::DoctorPrerequisite::Note(message) => {
                panic!("expected structured prerequisite, got note: {message}");
            }
        }
    }

    #[test]
    fn linux_backend_invokes_systemd_run_with_resource_properties_via_shim() {
        use std::{env, fs, os::unix::fs::PermissionsExt};

        let temp = tempfile::tempdir().unwrap();
        let shim_dir = temp.path().join("bin");
        fs::create_dir_all(&shim_dir).unwrap();
        let log_path = temp.path().join("argv.log");

        let shim_body = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{log}'\nwhile [ \"$#\" -gt 0 ]; do\n    arg=\"$1\"; shift\n    [ \"$arg\" = \"--\" ] && break\ndone\nexec \"$@\"\n",
            log = log_path.display()
        );
        let shim_path = shim_dir.join("systemd-run");
        fs::write(&shim_path, shim_body).unwrap();
        let mut perms = fs::metadata(&shim_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&shim_path, perms).unwrap();

        let original_path = env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{}", shim_dir.display(), original_path);

        let assert = assert_cmd::Command::cargo_bin("scaler")
            .unwrap()
            .env("PATH", &new_path)
            .env("SCALER_FORCE_BACKEND", "linux_systemd")
            .args([
                "run",
                "--cpu",
                "0.5c",
                "--mem",
                "64m",
                "--",
                "/bin/echo",
                "ok",
            ])
            .assert();

        assert.success();

        let recorded = fs::read_to_string(&log_path).unwrap();
        assert!(recorded.contains("--user"), "argv: {recorded}");
        assert!(recorded.contains("--scope"), "argv: {recorded}");
        assert!(
            recorded.contains("--property=CPUQuota=50%"),
            "argv: {recorded}"
        );
        assert!(
            recorded.contains("--property=MemoryMax=67108864"),
            "argv: {recorded}"
        );
        assert!(
            recorded.contains("--property=MemorySwapMax=0"),
            "argv: {recorded}"
        );
    }
}
