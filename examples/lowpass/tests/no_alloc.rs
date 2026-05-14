//! Pin the realtime no-allocation invariant for this example's
//! `process_io`.
//!
//! Installs [`assert_no_alloc::AllocDisabler`] as the test binary's
//! global allocator and drives [`LowPass`]'s `process_io` through a
//! `DriverInstance` inside an [`assert_no_alloc::assert_no_alloc`]
//! guard. A single allocation on the IO path aborts the test.
//!
//! `LowPass` is the stateful example — its one-pole filter advances
//! per-channel memory every `WriteMix` cycle. That state update, and
//! the `ReadInput` pass-through, both run inside the guard.

use std::sync::Arc;

use assert_no_alloc::{assert_no_alloc, AllocDisabler};

use lowpass::LowPass;
use tympan_aspl::driver::{AnyDriver, DriverInstance};
use tympan_aspl::io::{IoBuffer, IoOperation, Timestamp};
use tympan_aspl::RealtimeContext;

#[global_allocator]
static A: AllocDisabler = AllocDisabler;

#[test]
fn process_io_is_allocation_free() {
    let driver: Arc<dyn AnyDriver> = Arc::new(DriverInstance::<LowPass>::new());
    driver.initialize().unwrap();
    driver.start_io().unwrap();

    const FRAMES: usize = 256;
    let input: Vec<f32> = (0..FRAMES)
        .map(|i| (i as f32 / FRAMES as f32) * 2.0 - 1.0)
        .collect();
    let mut filtered = vec![0.0_f32; FRAMES];
    let mut passed = vec![0.0_f32; FRAMES];

    // SAFETY: a pure-logic integration test is case (2) of the
    // `RealtimeContext::new_unchecked` contract.
    let rt = unsafe { RealtimeContext::new_unchecked() };

    // The filter's per-channel memory advances every `WriteMix`
    // cycle — all of that state evolution runs inside the guard,
    // alongside the `ReadInput` pass-through.
    assert_no_alloc(|| {
        for _ in 0..64 {
            let mut w = IoBuffer::new(
                Timestamp::ZERO,
                IoOperation::WRITE_MIX,
                &input,
                &mut filtered,
            );
            driver.process_io(&rt, &mut w).unwrap();
            let mut r = IoBuffer::new(
                Timestamp::ZERO,
                IoOperation::READ_INPUT,
                &input,
                &mut passed,
            );
            driver.process_io(&rt, &mut r).unwrap();
        }
    });

    // Verify correctness outside the guard: the filter produced
    // finite samples, and `ReadInput` passed through unchanged.
    assert!(
        filtered.iter().all(|s| s.is_finite()),
        "the filtered output must be finite"
    );
    assert_eq!(passed, input, "ReadInput must pass through unfiltered");

    driver.stop_io().unwrap();
}
