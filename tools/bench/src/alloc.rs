//! A counting global allocator, so each run can report the allocations in its timed region.

use std::alloc::System;

use stats_alloc::{INSTRUMENTED_SYSTEM, StatsAlloc};

#[global_allocator]
static ALLOCATOR: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

/// What has been allocated since the process started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    /// Calls that allocated or grew a block.
    pub(crate) allocations: usize,
    /// Bytes those calls asked for. A grown block counts its growth.
    pub(crate) bytes: usize,
}

/// The counts now.
pub(crate) fn counts() -> Counts {
    let stats = ALLOCATOR.stats();
    Counts {
        allocations: stats.allocations + stats.reallocations,
        bytes: stats.bytes_allocated,
    }
}

impl Counts {
    /// What was allocated between `earlier` and `self`.
    pub(crate) fn since(self, earlier: Counts) -> Counts {
        Counts {
            allocations: self.allocations.saturating_sub(earlier.allocations),
            bytes: self.bytes.saturating_sub(earlier.bytes),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use super::*;

    #[test]
    fn counts_the_allocations_in_a_region() {
        let before = counts();
        let boxed = black_box(Box::new([0_u8; 100]));
        let during = counts().since(before);
        drop(boxed);
        // Other test threads may allocate too, so the count is at least ours.
        assert!(during.allocations >= 1, "{during:?}");
        assert!(during.bytes >= 100, "{during:?}");
    }

    #[test]
    fn a_region_without_allocation_counts_nothing_of_its_own() {
        let mut total = 0_u64;
        let before = counts();
        for n in 0..1000_u64 {
            total = black_box(total + n);
        }
        let during = counts().since(before);
        assert_eq!(total, 499_500);
        // Test threads that run alongside may allocate, so the count is small and not always zero.
        assert!(during.allocations < 1000, "{during:?}");
    }

    #[test]
    fn growing_a_vector_counts() {
        let before = counts();
        let mut grown: Vec<u8> = Vec::new();
        for n in 0..4096_u32 {
            grown.push(u8::try_from(n % 256).unwrap());
        }
        black_box(&grown);
        let during = counts().since(before);
        assert!(during.allocations >= 2, "{during:?}");
        assert!(during.bytes >= 4096, "{during:?}");
    }
}
