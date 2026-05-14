//! Reference AudioServerPlugin: a stereo virtual loopback device.
//!
//! [`MinimalLoopback`] is the smallest interesting driver — a
//! virtual device whose output is routed straight back to its
//! input, so anything played into it can be captured from it. It is
//! the macOS analogue of `tympan-apo`'s `passthrough` example and
//! `tympan-ladspa`'s `gain` example: a template a real driver is
//! grown from by replacing [`MinimalLoopback::process_io`] with the
//! desired DSP.
//!
//! ## What this example shows
//!
//! - Implementing [`Driver`] with the three identity constants and
//!   the [`Driver::new`] / [`Driver::device`] / [`Driver::process_io`]
//!   methods.
//! - Describing a loopback device with a [`DeviceSpec`] carrying
//!   both an input and an output [`StreamSpec`] in the canonical
//!   float32 format.
//! - A realtime-safe `process_io` body: a single `copy_from_slice`,
//!   no allocation, no locks.
//!
//! ## Building the `.driver` bundle
//!
//! This crate is an `rlib` today — the cross-platform `Driver`
//! implementation builds and is unit-tested on every host. Producing
//! the loadable `MinimalLoopback.driver` bundle needs two things
//! that land with the `raw` FFI bridge (see
//! `docs/decisions/0001-ci-verification-strategy.md`):
//!
//! 1. switching `crate-type` to `["cdylib"]` in `Cargo.toml`, and
//! 2. adding `tympan_aspl::plugin_entry!(MinimalLoopback);` at this
//!    crate's root to emit the CFPlugIn factory symbol.
//!
//! The committed `Info.plist` alongside this file already describes
//! the intended bundle, and Tier 2 CI `plutil`-lints it.

use tympan_aspl::{DeviceSpec, Driver, IoBuffer, RealtimeContext, StreamFormat, StreamSpec};

/// The stable device UID. Must not change across launches — the
/// system keeps per-device settings keyed on it.
pub const DEVICE_UID: &str = "com.tympan.aspl.MinimalLoopback";

/// Sample rate the virtual device runs at.
pub const SAMPLE_RATE: f64 = 48_000.0;

/// Channel count for both the input and the output stream.
pub const CHANNELS: u32 = 2;

/// A stereo virtual loopback device.
///
/// The struct is a zero-sized marker: the loopback has no
/// per-instance processing state, since [`Self::process_io`] just
/// copies its input to its output. A driver that needed state
/// (filter coefficients, a delay line, …) would carry it here and
/// pre-allocate it in [`Driver::start_io`].
pub struct MinimalLoopback;

impl Driver for MinimalLoopback {
    const NAME: &'static str = "Tympan Minimal Loopback";
    const MANUFACTURER: &'static str = "tympan-aspl";
    const VERSION: &'static str = "0.1.0";

    fn new() -> Self {
        Self
    }

    fn device(&self) -> DeviceSpec {
        let format = StreamFormat::float32(SAMPLE_RATE, CHANNELS);
        DeviceSpec::new(DEVICE_UID, "Minimal Loopback", Self::MANUFACTURER)
            .with_sample_rate(SAMPLE_RATE)
            .with_input(StreamSpec::input(format))
            .with_output(StreamSpec::output(format))
    }

    fn process_io(&mut self, _rt: &RealtimeContext, buffer: &mut IoBuffer<'_>) {
        // Loopback: route the device's input straight to its
        // output. A single `copy_from_slice` — allocation-free and
        // lock-free, so the realtime invariants hold.
        let n = buffer.output.len().min(buffer.input.len());
        buffer.output[..n].copy_from_slice(&buffer.input[..n]);
        // If the HAL handed us a longer output than input this
        // cycle, pad the tail with silence rather than leaving it
        // undefined.
        buffer.output[n..].fill(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tympan_aspl::driver::DriverInstance;
    use tympan_aspl::io::{IoOperation, Timestamp};

    fn rt() -> RealtimeContext {
        // Safety: a pure-logic unit test is case (2) of the
        // `RealtimeContext::new_unchecked` contract.
        unsafe { RealtimeContext::new_unchecked() }
    }

    #[test]
    fn device_spec_is_a_stereo_loopback() {
        let driver = MinimalLoopback::new();
        let spec = driver.device();
        assert_eq!(spec.uid(), DEVICE_UID);
        assert_eq!(spec.sample_rate(), SAMPLE_RATE);
        assert!(spec.is_loopback());
        assert_eq!(spec.input().unwrap().channels(), CHANNELS);
        assert_eq!(spec.output().unwrap().channels(), CHANNELS);
        assert!(spec.output().unwrap().format().is_canonical());
    }

    #[test]
    fn process_io_copies_input_to_output() {
        let driver = DriverInstance::<MinimalLoopback>::new();
        driver.initialize().unwrap();
        driver.start_io().unwrap();

        let input = [0.1_f32, -0.2, 0.3, -0.4, 0.5, -0.6];
        let mut output = [0.0_f32; 6];
        let rt = rt();
        let mut buffer = IoBuffer::new(
            Timestamp::ZERO,
            IoOperation::PROCESS_OUTPUT,
            &input,
            &mut output,
        );
        driver.process_io(&rt, &mut buffer).unwrap();
        assert_eq!(output, input);

        driver.stop_io().unwrap();
    }

    #[test]
    fn process_io_pads_a_longer_output_with_silence() {
        let mut driver = MinimalLoopback::new();
        let input = [1.0_f32, 1.0];
        let mut output = [9.0_f32; 4];
        let rt = rt();
        let mut buffer = IoBuffer::new(
            Timestamp::ZERO,
            IoOperation::PROCESS_OUTPUT,
            &input,
            &mut output,
        );
        driver.process_io(&rt, &mut buffer);
        assert_eq!(output, [1.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn identity_constants_are_wired_through() {
        let info = DriverInstance::<MinimalLoopback>::new().info();
        assert_eq!(info.name, "Tympan Minimal Loopback");
        assert_eq!(info.manufacturer, "tympan-aspl");
        assert_eq!(info.version, "0.1.0");
    }
}
