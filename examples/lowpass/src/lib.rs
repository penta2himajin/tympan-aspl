//! Reference AudioServerPlugin: a stateful one-pole low-pass
//! virtual device.
//!
//! [`LowPass`] is the example with genuine per-instance *processing
//! state*. Where `minimal-loopback` is stateless and `gain` carries
//! only fixed configuration, this driver carries a one-pole
//! low-pass filter's running memory — one sample per channel — and
//! evolves it every IO cycle.
//!
//! ## What this example shows
//!
//! - **Per-instance processing state.** The filter memory — the
//!   `prev` field of [`LowPass`] — lives in the struct, is created
//!   by [`Driver::new`], and persists across `process_io` calls;
//!   that is what makes the filter a filter.
//! - **Resetting state in [`Driver::start_io`].** A fresh IO
//!   session must not hear the tail of the previous one, so
//!   `start_io` clears the filter memory. `start_io` is also where
//!   a driver may *allocate* — allocation there is allowed,
//!   allocation in `process_io` is not.
//! - **Direction-aware processing.** Like the `gain` example, the
//!   filter is applied on the `WriteMix` operation only (audio
//!   entering the device) and passed through on `ReadInput`, so a
//!   sample crossing the loopback is filtered exactly once.
//! - A realtime-safe `process_io` body: a multiply-add recurrence,
//!   no allocation, no locks.
//!
//! ## The filter
//!
//! A one-pole low-pass: `y[n] = y[n-1] + α·(x[n] − y[n-1])`, run
//! independently per channel. [`ALPHA`] is the smoothing
//! coefficient — smaller is a lower cutoff.
//!
//! [`plugin_entry!`]: tympan_aspl::plugin_entry

use tympan_aspl::bundle::plist::BundleConfig;
use tympan_aspl::error::OsStatus;
use tympan_aspl::{
    DeviceSpec, Driver, IoBuffer, IoOperation, RealtimeContext, StreamFormat, StreamSpec,
};

/// The stable device UID. Must not change across launches — the
/// system keeps per-device settings keyed on it.
pub const DEVICE_UID: &str = "com.tympan.aspl.LowPass";

/// Sample rate the virtual device runs at.
pub const SAMPLE_RATE: f64 = 48_000.0;

/// Channel count for both the input and the output stream.
pub const CHANNELS: u32 = 2;

/// The one-pole smoothing coefficient, `0 < α ≤ 1`. `0.2` is a
/// gentle low-pass; closer to `1.0` lets more high frequency
/// through, closer to `0.0` is a heavier roll-off.
pub const ALPHA: f32 = 0.2;

/// The CFPlugIn factory UUID for this driver's bundle — the key of
/// the `Info.plist`'s `CFPlugInFactories` dictionary. Unique per
/// driver.
pub const FACTORY_UUID: &str = "212C67F0-C56F-4D0F-BF0E-7067DAAECA01";

/// A stateful one-pole low-pass virtual loopback device.
///
/// The struct carries the filter's running memory — the previous
/// output sample for each channel — which [`Self::process_io`]
/// reads and updates every cycle.
pub struct LowPass {
    /// The previous filtered sample, per channel. Pre-sized to
    /// [`CHANNELS`]; reset to silence by [`Driver::start_io`].
    prev: [f32; CHANNELS as usize],
}

impl LowPass {
    /// The [`BundleConfig`] describing this driver's `.driver`
    /// bundle. The committed `Info.plist` is exactly
    /// [`generate`](tympan_aspl::bundle::plist::generate)`(&LowPass::bundle_config())`
    /// — the `committed_info_plist_matches_the_generator` test
    /// enforces it.
    #[must_use]
    pub const fn bundle_config() -> BundleConfig {
        BundleConfig::new(DEVICE_UID, FACTORY_UUID, "TympanAsplDriverFactory")
            .with_bundle_name("Low Pass")
            .with_executable("LowPass")
            .with_version("0.1.0")
    }
}

impl Driver for LowPass {
    const NAME: &'static str = "Tympan Low Pass";
    const MANUFACTURER: &'static str = "tympan-aspl";
    const VERSION: &'static str = "0.1.0";

    fn new() -> Self {
        Self {
            prev: [0.0; CHANNELS as usize],
        }
    }

    fn device(&self) -> DeviceSpec {
        let format = StreamFormat::float32(SAMPLE_RATE, CHANNELS);
        DeviceSpec::new(DEVICE_UID, "Low Pass", Self::MANUFACTURER)
            .with_sample_rate(SAMPLE_RATE)
            .with_input(StreamSpec::input(format))
            .with_output(StreamSpec::output(format))
    }

    fn start_io(&mut self) -> Result<(), OsStatus> {
        // A fresh IO session starts from silence — clear the filter
        // memory so it does not carry the tail of the previous run.
        self.prev = [0.0; CHANNELS as usize];
        Ok(())
    }

    fn process_io(&mut self, _rt: &RealtimeContext, buffer: &mut IoBuffer<'_>) {
        let n = buffer.output.len().min(buffer.input.len());
        match buffer.operation {
            // `WriteMix` — audio entering the device. Run it through
            // the one-pole filter, advancing the per-channel memory.
            IoOperation::WRITE_MIX => {
                for i in 0..n {
                    let channel = i % CHANNELS as usize;
                    let x = buffer.input[i];
                    // y[n] = y[n-1] + α·(x[n] − y[n-1])
                    self.prev[channel] += ALPHA * (x - self.prev[channel]);
                    buffer.output[i] = self.prev[channel];
                }
            }
            // `ReadInput` (or anything else) — audio leaving the
            // device. Pass it through so the filter runs once across
            // the loopback.
            _ => buffer.output[..n].copy_from_slice(&buffer.input[..n]),
        }
        // Pad a longer output with silence rather than leaving its
        // tail undefined.
        buffer.output[n..].fill(0.0);
    }
}

// Emit the `TympanAsplDriverFactory` CFPlugIn factory entry point —
// the symbol `coreaudiod` resolves from the bundle's `Info.plist`.
tympan_aspl::plugin_entry!(LowPass);

#[cfg(test)]
mod tests {
    use super::*;
    use tympan_aspl::bundle::plist::generate;
    use tympan_aspl::driver::DriverInstance;
    use tympan_aspl::io::Timestamp;

    fn rt() -> RealtimeContext {
        // Safety: a pure-logic unit test is case (2) of the
        // `RealtimeContext::new_unchecked` contract.
        unsafe { RealtimeContext::new_unchecked() }
    }

    fn buffer<'a>(operation: IoOperation, input: &'a [f32], output: &'a mut [f32]) -> IoBuffer<'a> {
        IoBuffer::new(Timestamp::ZERO, operation, input, output)
    }

    #[test]
    fn device_spec_is_a_stereo_loopback() {
        let driver = LowPass::new();
        let spec = driver.device();
        assert_eq!(spec.uid(), DEVICE_UID);
        assert_eq!(spec.sample_rate(), SAMPLE_RATE);
        assert!(spec.is_loopback());
        assert_eq!(spec.input().unwrap().channels(), CHANNELS);
        assert_eq!(spec.output().unwrap().channels(), CHANNELS);
    }

    #[test]
    fn write_mix_low_pass_filters_toward_the_input() {
        let mut driver = LowPass::new();
        // A constant input: the one-pole output rises monotonically
        // toward it, never overshooting, and never reaching it in a
        // finite number of steps.
        let input = [1.0_f32; 8];
        let mut output = [0.0_f32; 8];
        driver.process_io(
            &rt(),
            &mut buffer(IoOperation::WRITE_MIX, &input, &mut output),
        );
        // Channel 0 sees samples 0,2,4,6; channel 1 sees 1,3,5,7 —
        // both ramp identically from the same zero start.
        for pair in output.chunks_exact(2) {
            assert_eq!(pair[0], pair[1], "the two channels filter in lockstep");
        }
        let ch0: Vec<f32> = output.iter().step_by(2).copied().collect();
        for w in ch0.windows(2) {
            assert!(w[1] > w[0], "the filtered output rises toward the input");
            assert!(w[1] < 1.0, "and never overshoots a constant input");
        }
        // First step is exactly α·(1 − 0).
        assert!((ch0[0] - ALPHA).abs() < 1.0e-6);
    }

    #[test]
    fn read_input_passes_through_unfiltered() {
        let mut driver = LowPass::new();
        let input = [0.2_f32, -0.4, 1.0, -1.0];
        let mut output = [0.0_f32; 4];
        driver.process_io(
            &rt(),
            &mut buffer(IoOperation::READ_INPUT, &input, &mut output),
        );
        assert_eq!(output, input);
        // A pass-through must not disturb the filter memory.
        assert_eq!(driver.prev, [0.0, 0.0]);
    }

    #[test]
    fn filter_memory_persists_across_cycles() {
        let mut driver = LowPass::new();
        let input = [1.0_f32, 1.0];
        let mut first = [0.0_f32; 2];
        let mut second = [0.0_f32; 2];
        driver.process_io(
            &rt(),
            &mut buffer(IoOperation::WRITE_MIX, &input, &mut first),
        );
        driver.process_io(
            &rt(),
            &mut buffer(IoOperation::WRITE_MIX, &input, &mut second),
        );
        // The second cycle continues from where the first left off,
        // so its output is strictly higher.
        assert!(second[0] > first[0]);
    }

    #[test]
    fn start_io_resets_the_filter_memory() {
        let driver = DriverInstance::<LowPass>::new();
        driver.initialize().unwrap();
        driver.start_io().unwrap();
        let input = [1.0_f32, 1.0];
        let mut warmup = [0.0_f32; 2];
        driver
            .process_io(
                &rt(),
                &mut buffer(IoOperation::WRITE_MIX, &input, &mut warmup),
            )
            .unwrap();
        assert!(warmup[0] > 0.0, "the filter has accumulated some memory");

        // Stopping and restarting clears that memory: the first
        // sample of the new session is α·(1 − 0) again.
        driver.stop_io().unwrap();
        driver.start_io().unwrap();
        let mut fresh = [0.0_f32; 2];
        driver
            .process_io(
                &rt(),
                &mut buffer(IoOperation::WRITE_MIX, &input, &mut fresh),
            )
            .unwrap();
        assert!(
            (fresh[0] - ALPHA).abs() < 1.0e-6,
            "start_io reset the memory"
        );
        driver.stop_io().unwrap();
    }

    #[test]
    fn process_io_pads_a_longer_output_with_silence() {
        let mut driver = LowPass::new();
        let input = [1.0_f32, 1.0];
        let mut output = [9.0_f32; 4];
        driver.process_io(
            &rt(),
            &mut buffer(IoOperation::WRITE_MIX, &input, &mut output),
        );
        assert_eq!(output[2..], [0.0, 0.0]);
    }

    #[test]
    fn identity_constants_are_wired_through() {
        let info = DriverInstance::<LowPass>::new().info();
        assert_eq!(info.name, "Tympan Low Pass");
        assert_eq!(info.manufacturer, "tympan-aspl");
        assert_eq!(info.version, "0.1.0");
    }

    #[test]
    fn committed_info_plist_matches_the_generator() {
        // The committed `Info.plist` must stay byte-identical to
        // what `bundle::plist::generate` emits for this driver.
        assert_eq!(
            generate(&LowPass::bundle_config()),
            include_str!("../Info.plist")
        );
    }
}
