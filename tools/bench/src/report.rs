//! The plain-text tables the command prints. The columns have fixed widths and the rows come in the
//! order the workloads ran, so two reports diff line by line.

use std::fmt::Write as _;

use crate::stats::{Spread, Summary};

/// One row of the main table.
pub(crate) struct Row<'a> {
    pub(crate) workload: &'a str,
    pub(crate) stack: &'a str,
    pub(crate) summary: Summary,
    pub(crate) spread: Spread,
}

/// The table of medians, 95th percentiles, bytes and allocations, and under it, when there were
/// several repeats, how the median of each repeat varied.
pub(crate) fn tables(rows: &[Row<'_>]) -> String {
    let mut out = String::new();
    // Writing to a `String` can't fail, so the results are dropped.
    let _ = writeln!(
        out,
        "{:<16} {:<8} {:>6} {:>13} {:>13} {:>13} {:>13} {:>10} {:>9} {:>6} {:>11}",
        "workload",
        "stack",
        "frames",
        "render_us_p50",
        "render_us_p95",
        "encode_us_p50",
        "encode_us_p95",
        "bytes_mean",
        "bytes_max",
        "allocs",
        "alloc_bytes"
    );
    for row in rows {
        let s = &row.summary;
        let _ = writeln!(
            out,
            "{:<16} {:<8} {:>6} {:>13.1} {:>13.1} {:>13.1} {:>13.1} {:>10.1} {:>9} {:>6.1} {:>11.0}",
            row.workload,
            row.stack,
            s.frames,
            s.render_p50,
            s.render_p95,
            s.encode_p50,
            s.encode_p95,
            s.bytes_mean,
            s.bytes_max,
            s.allocations,
            s.alloc_bytes
        );
    }
    if rows.first().is_some_and(|row| row.spread.repeats > 1) {
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "{:<16} {:<8} {:>7} {:>13} {:>13} {:>10} {:>13} {:>13} {:>10}",
            "workload",
            "stack",
            "repeats",
            "render_us_min",
            "render_us_max",
            "render_pct",
            "encode_us_min",
            "encode_us_max",
            "encode_pct"
        );
        for row in rows {
            let s = &row.spread;
            let _ = writeln!(
                out,
                "{:<16} {:<8} {:>7} {:>13.1} {:>13.1} {:>10.1} {:>13.1} {:>13.1} {:>10.1}",
                row.workload,
                row.stack,
                s.repeats,
                s.render_min,
                s.render_max,
                Spread::percent(s.render_min, s.render_max),
                s.encode_min,
                s.encode_max,
                Spread::percent(s.encode_min, s.encode_max)
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary() -> Summary {
        Summary {
            frames: 100,
            render_p50: 12.34,
            render_p95: 20.0,
            encode_p50: 30.04,
            encode_p95: 45.5,
            bytes_mean: 201.25,
            bytes_max: 640,
            allocations: 0.0,
            alloc_bytes: 0.0,
        }
    }

    fn spread(repeats: usize) -> Spread {
        Spread {
            repeats,
            render_min: 11.0,
            render_max: 13.0,
            encode_min: 28.0,
            encode_max: 33.6,
        }
    }

    #[test]
    fn rows_line_up_under_the_header() {
        let rows = [
            Row {
                workload: "idle",
                stack: "ratatui",
                summary: summary(),
                spread: spread(1),
            },
            Row {
                workload: "full-redraw",
                stack: "ratatui",
                summary: summary(),
                spread: spread(1),
            },
        ];
        let table = tables(&rows);
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines.len(), 3, "{table}");
        assert_eq!(
            lines[0],
            "workload         stack    frames render_us_p50 render_us_p95 encode_us_p50 encode_us_p95 bytes_mean bytes_max allocs alloc_bytes"
        );
        assert_eq!(
            lines[1],
            "idle             ratatui     100          12.3          20.0          30.0          45.5      201.2       640    0.0           0"
        );
        assert!(
            lines.iter().all(|line| line.len() == lines[0].len()),
            "{table}"
        );
    }

    #[test]
    fn several_repeats_add_the_spread() {
        let rows = [Row {
            workload: "idle",
            stack: "ratatui",
            summary: summary(),
            spread: spread(5),
        }];
        let table = tables(&rows);
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines.len(), 5, "{table}");
        assert_eq!(lines[2], "");
        assert_eq!(
            lines[3],
            "workload         stack    repeats render_us_min render_us_max render_pct encode_us_min encode_us_max encode_pct"
        );
        assert_eq!(
            lines[4],
            "idle             ratatui        5          11.0          13.0       18.2          28.0          33.6       20.0"
        );
    }

    #[test]
    fn no_rows_print_only_the_header() {
        assert_eq!(tables(&[]).lines().count(), 1);
    }
}
