use std::process::Command;
use std::time::SystemTime;

use anyhow::{Context, Result};

use crate::core::Sample;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PsRow {
    pub pid: u32,
    pub ppid: u32,
    pub rss_kib: u64,
    pub cpu_percent: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct AggregatedMetrics {
    pub rss_bytes: u64,
    pub cpu_percent: f32,
    pub process_count: u32,
}

pub(crate) fn parse_ps_table(input: &str) -> Vec<PsRow> {
    input
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse::<u32>().ok()?;
            let ppid = fields.next()?.parse::<u32>().ok()?;
            let rss_kib = fields.next()?.parse::<u64>().ok()?;
            let cpu_percent = fields.next()?.parse::<f32>().ok()?;
            Some(PsRow {
                pid,
                ppid,
                rss_kib,
                cpu_percent,
            })
        })
        .collect()
}

pub(crate) fn aggregate_descendants(rows: &[PsRow], root_pid: u32) -> AggregatedMetrics {
    // If the root pid disappeared between our process_state lookup and the
    // ps snapshot, return zero metrics rather than fabricating partial
    // numbers from any orphaned descendants that are still in the table.
    if !rows.iter().any(|r| r.pid == root_pid) {
        return AggregatedMetrics::default();
    }

    let mut included = std::collections::HashSet::new();
    let mut frontier = vec![root_pid];

    while let Some(pid) = frontier.pop() {
        if !included.insert(pid) {
            continue;
        }
        for row in rows {
            if row.ppid == pid && !included.contains(&row.pid) {
                frontier.push(row.pid);
            }
        }
    }

    let mut metrics = AggregatedMetrics::default();
    for row in rows {
        if included.contains(&row.pid) {
            metrics.rss_bytes = metrics.rss_bytes.saturating_add(row.rss_kib * 1024);
            metrics.cpu_percent += row.cpu_percent;
            metrics.process_count += 1;
        }
    }
    metrics
}

pub fn sample_process_tree(root_pid: u32) -> Result<Sample> {
    let output = Command::new("ps")
        .args(["-e", "-o", "pid=,ppid=,rss=,%cpu="])
        .output()
        .with_context(|| format!("failed to invoke ps for pid {root_pid}"))?;
    anyhow::ensure!(
        output.status.success(),
        "ps exited with non-success status while sampling pid {root_pid}"
    );

    let table = String::from_utf8_lossy(&output.stdout);
    let rows = parse_ps_table(&table);
    let metrics = aggregate_descendants(&rows, root_pid);

    Ok(Sample {
        captured_at: SystemTime::now(),
        cpu_percent: metrics.cpu_percent,
        memory_bytes: metrics.rss_bytes,
        peak_memory_bytes: Some(metrics.rss_bytes),
        child_process_count: Some(metrics.process_count),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_ps_lines() {
        let input = "  100   1  4096  3.5\n  101 100  2048  1.0\n";
        let rows = parse_ps_table(input);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].pid, 100);
        assert_eq!(rows[0].ppid, 1);
        assert_eq!(rows[0].rss_kib, 4096);
        assert!((rows[0].cpu_percent - 3.5).abs() < 1e-6);
        assert_eq!(rows[1].pid, 101);
        assert_eq!(rows[1].ppid, 100);
    }

    #[test]
    fn ignores_unparseable_rows() {
        let input = "header line\n100 1 4096 3.5\n  garbage\n";
        let rows = parse_ps_table(input);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].pid, 100);
    }

    #[test]
    fn aggregates_only_descendants_of_root() {
        let rows = vec![
            PsRow {
                pid: 1,
                ppid: 0,
                rss_kib: 100,
                cpu_percent: 0.0,
            },
            PsRow {
                pid: 100,
                ppid: 1,
                rss_kib: 4096,
                cpu_percent: 5.0,
            },
            PsRow {
                pid: 200,
                ppid: 100,
                rss_kib: 2048,
                cpu_percent: 2.0,
            },
            PsRow {
                pid: 201,
                ppid: 100,
                rss_kib: 1024,
                cpu_percent: 1.0,
            },
            PsRow {
                pid: 300,
                ppid: 200,
                rss_kib: 512,
                cpu_percent: 0.5,
            },
            PsRow {
                pid: 999,
                ppid: 1,
                rss_kib: 10000,
                cpu_percent: 50.0,
            },
        ];

        let metrics = aggregate_descendants(&rows, 100);

        assert_eq!(metrics.process_count, 4);
        assert_eq!(metrics.rss_bytes, (4096 + 2048 + 1024 + 512) * 1024);
        assert!((metrics.cpu_percent - 8.5).abs() < 1e-3);
    }

    #[test]
    fn aggregates_root_only_when_no_children() {
        let rows = vec![PsRow {
            pid: 100,
            ppid: 1,
            rss_kib: 4096,
            cpu_percent: 5.0,
        }];
        let metrics = aggregate_descendants(&rows, 100);
        assert_eq!(metrics.process_count, 1);
        assert_eq!(metrics.rss_bytes, 4096 * 1024);
    }

    #[test]
    fn aggregates_zero_when_root_missing() {
        let rows = vec![PsRow {
            pid: 200,
            ppid: 100,
            rss_kib: 1024,
            cpu_percent: 1.0,
        }];
        let metrics = aggregate_descendants(&rows, 100);
        assert_eq!(metrics.process_count, 0);
        assert_eq!(metrics.rss_bytes, 0);
        assert!((metrics.cpu_percent - 0.0).abs() < 1e-6);
    }

    #[test]
    fn handles_ppid_cycle_without_hanging() {
        // Malformed input where pid 100 claims pid 101 as parent and pid 101
        // claims pid 100 as parent. The included.insert dedup must prevent
        // infinite traversal.
        let rows = vec![
            PsRow {
                pid: 100,
                ppid: 101,
                rss_kib: 1024,
                cpu_percent: 1.0,
            },
            PsRow {
                pid: 101,
                ppid: 100,
                rss_kib: 512,
                cpu_percent: 0.5,
            },
        ];
        let metrics = aggregate_descendants(&rows, 100);
        assert_eq!(metrics.process_count, 2);
        assert_eq!(metrics.rss_bytes, (1024 + 512) * 1024);
        assert!((metrics.cpu_percent - 1.5).abs() < 1e-3);
    }
}
