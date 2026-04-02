#[cfg(target_os = "macos")]
mod macos_tests {
    use std::ffi::OsString;

    use scaler::{
        backend::macos_taskpolicy::{MacosProbe, build_taskpolicy_argv, detect_macos_capabilities},
        core::{
            BackendKind, CapabilityLevel, CpuLimit, InteractiveMode, LaunchPlan, MemoryLimit,
            Platform, ResourceSpec, ShellKind,
        },
    };

    #[test]
    fn macos_backend_marks_cpu_best_effort() {
        let report = detect_macos_capabilities(
            MacosProbe {
                has_taskpolicy: true,
                has_renice: true,
                has_memory_support: true,
                has_pty_support: true,
                platform_version_supported: true,
            },
            InteractiveMode::Auto,
        );

        assert_eq!(report.platform, Platform::Macos);
        assert_eq!(report.backend, BackendKind::MacosTaskpolicy);
        assert_eq!(report.cpu, CapabilityLevel::BestEffort);
    }

    #[test]
    fn macos_detect_reports_missing_renice() {
        let report = detect_macos_capabilities(
            MacosProbe {
                has_taskpolicy: true,
                has_renice: false,
                has_memory_support: true,
                has_pty_support: true,
                platform_version_supported: true,
            },
            InteractiveMode::Auto,
        );

        assert_eq!(report.backend_state, CapabilityLevel::BestEffort);
        assert_eq!(report.cpu, CapabilityLevel::BestEffort);
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("renice"))
        );
    }

    #[test]
    fn macos_detect_reports_missing_memory_support() {
        let report = detect_macos_capabilities(
            MacosProbe {
                has_taskpolicy: true,
                has_renice: true,
                has_memory_support: false,
                has_pty_support: true,
                platform_version_supported: true,
            },
            InteractiveMode::Auto,
        );

        assert_eq!(report.backend_state, CapabilityLevel::BestEffort);
        assert_eq!(report.memory, CapabilityLevel::Unavailable);
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("memory"))
        );
    }

    #[test]
    fn macos_detect_reports_missing_taskpolicy_and_pty_rules() {
        let missing_taskpolicy = detect_macos_capabilities(
            MacosProbe {
                has_taskpolicy: false,
                has_renice: true,
                has_memory_support: true,
                has_pty_support: true,
                platform_version_supported: true,
            },
            InteractiveMode::Auto,
        );
        assert_eq!(
            missing_taskpolicy.backend_state,
            CapabilityLevel::Unavailable
        );
        assert_eq!(missing_taskpolicy.cpu, CapabilityLevel::Unavailable);
        assert_eq!(missing_taskpolicy.memory, CapabilityLevel::Unavailable);
        assert_eq!(missing_taskpolicy.interactive, CapabilityLevel::Unavailable);
        assert!(
            missing_taskpolicy
                .warnings
                .iter()
                .any(|warning| warning.contains("taskpolicy"))
        );

        let no_pty_auto = detect_macos_capabilities(
            MacosProbe {
                has_taskpolicy: true,
                has_renice: true,
                has_memory_support: true,
                has_pty_support: false,
                platform_version_supported: true,
            },
            InteractiveMode::Auto,
        );
        assert_eq!(no_pty_auto.interactive, CapabilityLevel::BestEffort);

        let no_pty_always = detect_macos_capabilities(
            MacosProbe {
                has_taskpolicy: true,
                has_renice: true,
                has_memory_support: true,
                has_pty_support: false,
                platform_version_supported: true,
            },
            InteractiveMode::Always,
        );
        assert_eq!(no_pty_always.interactive, CapabilityLevel::Unavailable);
        assert!(
            no_pty_always
                .warnings
                .iter()
                .any(|warning| warning.contains("PTY"))
        );
    }

    #[test]
    fn macos_detect_reports_unsupported_platform_version() {
        let report = detect_macos_capabilities(
            MacosProbe {
                has_taskpolicy: true,
                has_renice: true,
                has_memory_support: true,
                has_pty_support: true,
                platform_version_supported: false,
            },
            InteractiveMode::Auto,
        );

        assert_eq!(report.backend_state, CapabilityLevel::Unavailable);
        assert_eq!(report.cpu, CapabilityLevel::Unavailable);
        assert_eq!(report.memory, CapabilityLevel::Unavailable);
        assert_eq!(report.interactive, CapabilityLevel::Unavailable);
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("version"))
        );
    }

    #[test]
    fn macos_command_builds_taskpolicy_argv() {
        let plan = LaunchPlan {
            argv: vec![OsString::from("echo"), OsString::from("ok")],
            resource_spec: ResourceSpec {
                cpu: Some(CpuLimit::from_centi_cores(100)),
                mem: Some(MemoryLimit::from_bytes(1_073_741_824)),
                interactive: InteractiveMode::Always,
                shell: None,
                monitor: true,
            },
            platform: Platform::Macos,
        };

        let argv = build_taskpolicy_argv(&plan).unwrap();
        let argv = argv
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(argv[0], "taskpolicy");
        assert!(argv.iter().any(|value| value == "-d"));
        assert!(argv.iter().any(|value| value == "-g"));
        assert!(argv.iter().any(|value| value == "-m"));
        assert_eq!(&argv[argv.len() - 2..], ["echo", "ok"]);
    }

    #[test]
    fn macos_command_wraps_shell_script_when_requested() {
        let plan = LaunchPlan {
            argv: vec![OsString::from("echo ok")],
            resource_spec: ResourceSpec {
                shell: Some(ShellKind::Sh),
                ..ResourceSpec::default()
            },
            platform: Platform::Macos,
        };

        let argv = build_taskpolicy_argv(&plan).unwrap();
        let argv = argv
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            &argv[argv.len() - 3..],
            ["sh".to_string(), "-lc".to_string(), "echo ok".to_string()]
        );
    }

    #[test]
    fn macos_detect_reports_best_effort_happy_path() {
        let report = detect_macos_capabilities(
            MacosProbe {
                has_taskpolicy: true,
                has_renice: true,
                has_memory_support: true,
                has_pty_support: true,
                platform_version_supported: true,
            },
            InteractiveMode::Auto,
        );

        assert_eq!(report.backend_state, CapabilityLevel::BestEffort);
        assert_eq!(report.cpu, CapabilityLevel::BestEffort);
        assert_eq!(report.memory, CapabilityLevel::BestEffort);
        assert_eq!(report.interactive, CapabilityLevel::BestEffort);
        assert!(report.warnings.is_empty());
    }
}
