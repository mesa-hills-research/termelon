//! Plays a workload through a stack and times every frame.

use std::hint::black_box;
use std::time::{Duration, Instant};

use crate::alloc;
use crate::stack;
use crate::workload::Workload;

/// What one frame cost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sample {
    /// Building the stack's buffer from the neutral frame, in nanoseconds.
    pub(crate) render_ns: u64,
    /// The diff and the encoder, with a resize before the frame if there was one, in nanoseconds.
    pub(crate) encode_ns: u64,
    /// Bytes written for the frame.
    pub(crate) bytes: usize,
    /// Allocations in the timed region.
    pub(crate) allocations: usize,
    /// Bytes those allocations asked for.
    pub(crate) alloc_bytes: usize,
}

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// Plays `workload` through the stack called `name` and returns a sample for each timed frame.
///
/// # Errors
///
/// Fails if the stack is unknown or the workload has no frames.
pub(crate) fn run(name: &str, workload: &Workload) -> Result<Vec<Sample>, String> {
    let first = workload
        .warmup
        .first()
        .or_else(|| workload.frames.first())
        .ok_or_else(|| format!("{} has no frames", workload.name))?;
    let mut size = (first.width(), first.height());
    let mut stack = stack::make(name, size.0, size.1)?;
    for frame in &workload.warmup {
        if (frame.width(), frame.height()) != size {
            size = (frame.width(), frame.height());
            stack.resize(size.0, size.1);
        }
        stack.render(frame);
        black_box(stack.present(frame.cursor).len());
    }
    let mut samples = Vec::with_capacity(workload.frames.len());
    for frame in &workload.frames {
        let resize = workload.repaint || (frame.width(), frame.height()) != size;
        let before = alloc::counts();
        let started = Instant::now();
        if resize {
            size = (frame.width(), frame.height());
            stack.resize(size.0, size.1);
        }
        let resized = Instant::now();
        stack.render(frame);
        let rendered = Instant::now();
        let bytes = black_box(stack.present(frame.cursor)).len();
        let done = Instant::now();
        let used = alloc::counts().since(before);
        samples.push(Sample {
            render_ns: nanos(rendered - resized),
            encode_ns: nanos(resized - started) + nanos(done - rendered),
            bytes,
            allocations: used.allocations,
            alloc_bytes: used.bytes,
        });
    }
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Frame, Style};
    use crate::workload::{build, from_frames};

    #[test]
    fn a_sample_for_every_timed_frame() {
        let workload = build("typing", 5).unwrap();
        let samples = run("ratatui", &workload).unwrap();
        assert_eq!(samples.len(), 5);
        assert!(samples.iter().all(|s| s.bytes > 0));
        assert!(samples.iter().all(|s| s.render_ns > 0 && s.encode_ns > 0));
    }

    #[test]
    fn the_warm_up_is_not_timed_so_a_small_change_costs_a_small_write() {
        let workload = build("typing", 5).unwrap();
        let samples = run("ratatui", &workload).unwrap();
        // The whole screen was drawn in the warm-up, so a typed character takes a few dozen bytes.
        assert!(samples.iter().all(|s| s.bytes < 200), "{samples:?}");
    }

    #[test]
    fn a_repainting_workload_writes_the_whole_screen_every_frame() {
        let workload = build("full-redraw", 2).unwrap();
        let samples = run("ratatui", &workload).unwrap();
        // 30,000 cells with a colour change at nearly every one.
        assert!(samples.iter().all(|s| s.bytes > 500_000), "{samples:?}");
    }

    #[test]
    fn a_change_of_size_is_a_resize() {
        let mut small = Frame::new(10, 3);
        small.put_str(0, 0, "ab", Style::default(), 10);
        let mut big = Frame::new(12, 4);
        big.put_str(0, 0, "cd", Style::default(), 12);
        let workload = from_frames("sizes", vec![small, big.clone(), big], false);
        let samples = run("ratatui", &workload).unwrap();
        assert_eq!(samples.len(), 2);
        assert!(samples[0].bytes > samples[1].bytes, "{samples:?}");
    }

    #[test]
    fn errors_say_what_is_wrong() {
        let workload = build("typing", 2).unwrap();
        assert!(
            run("nothing", &workload)
                .unwrap_err()
                .contains("unknown stack")
        );
        let empty = from_frames("empty", Vec::new(), false);
        assert!(run("ratatui", &empty).unwrap_err().contains("no frames"));
    }
}
