//! Pin the realtime no-allocation invariant for this example's
//! `process_io`.
//!
//! The framework's `tests/realtime_safety.rs` proves the *framework*
//! is allocation-free using a test-local fixture driver. This test
//! does the same for the *example driver itself*: it installs
//! [`assert_no_alloc::AllocDisabler`] as the test binary's global
//! allocator and drives [`MinimalLoopback`]'s `process_io` through a
//! `DriverInstance` inside an [`assert_no_alloc::assert_no_alloc`]
//! guard. A single allocation on the IO path aborts the test.
//!
//! Buffers and the `DriverInstance` are built *before* the guard —
//! the invariant is about `process_io`, not the test's own setup.

use std::sync::Arc;

use assert_no_alloc::{assert_no_alloc, AllocDisabler};

use minimal_loopback::MinimalLoopback;
use tympan_aspl::driver::{AnyDriver, DriverInstance};
use tympan_aspl::io::{IoBuffer, IoOperation, Timestamp};
use tympan_aspl::RealtimeContext;

#[global_allocator]
static A: AllocDisabler = AllocDisabler;

#[test]
fn process_io_is_allocation_free() {
    let driver: Arc<dyn AnyDriver> = Arc::new(DriverInstance::<MinimalLoopback>::new());
    driver.initialize().unwrap();
    driver.start_io().unwrap();

    const FRAMES: usize = 256;
    let input: Vec<f32> = (0..FRAMES)
        .map(|i| (i as f32 / FRAMES as f32) * 2.0 - 1.0)
        .collect();
    let mut output = vec![0.0_f32; FRAMES];

    // SAFETY: a pure-logic integration test is case (2) of the
    // `RealtimeContext::new_unchecked` contract.
    let rt = unsafe { RealtimeContext::new_unchecked() };

    assert_no_alloc(|| {
        for _ in 0..64 {
            let mut buffer =
                IoBuffer::new(Timestamp::ZERO, IoOperation::WRITE_MIX, &input, &mut output);
            driver.process_io(&rt, &mut buffer).unwrap();
        }
    });

    // Verify correctness outside the guard, where the assertion
    // machinery is free to allocate on failure.
    assert_eq!(
        output, input,
        "the loopback must copy its input to its output"
    );

    driver.stop_io().unwrap();
}
