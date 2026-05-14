//! Pin the realtime no-allocation invariant for this example's
//! `process_io`.
//!
//! Installs [`assert_no_alloc::AllocDisabler`] as the test binary's
//! global allocator and drives [`Gain`]'s `process_io` through a
//! `DriverInstance` inside an [`assert_no_alloc::assert_no_alloc`]
//! guard. A single allocation on the IO path aborts the test.
//!
//! `Gain::process_io` is direction-aware — it scales on `WriteMix`
//! and passes through on `ReadInput` — so both branches are driven
//! inside the guard.

use std::sync::Arc;

use assert_no_alloc::{assert_no_alloc, AllocDisabler};

use gain::{Gain, GAIN};
use tympan_aspl::driver::{AnyDriver, DriverInstance};
use tympan_aspl::io::{IoBuffer, IoOperation, Timestamp};
use tympan_aspl::RealtimeContext;

#[global_allocator]
static A: AllocDisabler = AllocDisabler;

#[test]
fn process_io_is_allocation_free() {
    let driver: Arc<dyn AnyDriver> = Arc::new(DriverInstance::<Gain>::new());
    driver.initialize().unwrap();
    driver.start_io().unwrap();

    const FRAMES: usize = 256;
    let input: Vec<f32> = (0..FRAMES)
        .map(|i| (i as f32 / FRAMES as f32) * 2.0 - 1.0)
        .collect();
    let mut scaled = vec![0.0_f32; FRAMES];
    let mut passed = vec![0.0_f32; FRAMES];

    // SAFETY: a pure-logic integration test is case (2) of the
    // `RealtimeContext::new_unchecked` contract.
    let rt = unsafe { RealtimeContext::new_unchecked() };

    // Both branches of the direction-aware `process_io` run inside
    // the guard: `WriteMix` scales, `ReadInput` passes through.
    assert_no_alloc(|| {
        for _ in 0..64 {
            let mut w = IoBuffer::new(Timestamp::ZERO, IoOperation::WRITE_MIX, &input, &mut scaled);
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

    // Verify correctness outside the guard: `WriteMix` scaled by the
    // gain, `ReadInput` passed through unchanged.
    for (i, &x) in input.iter().enumerate() {
        assert_eq!(scaled[i], x * GAIN, "WriteMix must scale by the gain");
        assert_eq!(passed[i], x, "ReadInput must pass through unscaled");
    }

    driver.stop_io().unwrap();
}
