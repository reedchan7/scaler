use std::time::SystemTime;

use scaler::core::{CapabilityReport, Sample, SummarySample};

#[test]
fn unsupported_capability_report_has_no_warnings() {
    let report = CapabilityReport::unsupported();

    assert!(report.warnings.is_empty());
}

#[test]
fn sample_contract_uses_concrete_numeric_fields() {
    let captured_at = SystemTime::UNIX_EPOCH;
    let sample = Sample {
        captured_at,
        cpu_percent: 12.5,
        memory_bytes: 1024,
        peak_memory_bytes: 2048,
        child_process_count: 3,
    };
    let summary = SummarySample {
        captured_at,
        cpu_percent: 6.25,
        memory_bytes: 512,
    };

    assert_eq!(sample.captured_at, captured_at);
    assert_eq!(sample.cpu_percent, 12.5);
    assert_eq!(sample.memory_bytes, 1024);
    assert_eq!(sample.peak_memory_bytes, 2048);
    assert_eq!(sample.child_process_count, 3);
    assert_eq!(summary.captured_at, captured_at);
    assert_eq!(summary.cpu_percent, 6.25);
    assert_eq!(summary.memory_bytes, 512);
}
