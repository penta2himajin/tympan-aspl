//! Reference AudioServerPlugin: a fixed-gain virtual loopback
//! device.
//!
//! [`Gain`] is one step up from `minimal-loopback`: instead of
//! routing its input straight to its output untouched, it applies a
//! fixed linear gain. It is the macOS analogue of `tympan-apo`'s
//! `gain` example and `tympan-ladspa`'s `gain` example.
//!
//! ## What this example shows
//!
//! - **Per-instance configuration.** The gain is held in a struct
//!   field, initialised by [`Driver::new`] — the place a real
//!   driver would read user settings, allocate buffers, or set up
//!   DSP coefficients.
//! - **Direction-aware processing.** A loopback device's
//!   [`Driver::process_io`] is called for *two* IO operations per
//!   client: `WriteMix` (audio entering the device) and `ReadInput`
//!   (audio leaving it). Applying a transform unconditionally would
//!   apply it twice. This driver branches on
//!   [`IoBuffer::operation`] and scales only on `WriteMix`, so the
//!   gain is applied exactly once — the device boosts everything
//!   played into it, and hands it back unchanged on the way out.
//! - A realtime-safe `process_io` body: a multiply-add loop, no
//!   allocation, no locks.
//!
//! ## The `.driver` bundle
//!
//! The crate builds as a `cdylib`: [`plugin_entry!`] emits the
//! `TympanAsplDriverFactory` CFPlugIn entry point. Wrapped with the
//! committed `Info.plist`, the cdylib is the loadable `Gain.driver`
//! bundle — see [`README.md`](https://github.com/penta2himajin/tympan-aspl/tree/main/examples/gain)
//! for the layout. The crate is also an `rlib` so its unit tests
//! link the [`Driver`] implementation directly.
//!
//! [`plugin_entry!`]: tympan_aspl::plugin_entry

use tympan_aspl::bundle::plist::BundleConfig;
use tympan_aspl::{
    DeviceSpec, Driver, IoBuffer, IoOperation, RealtimeContext, StreamFormat, StreamSpec,
};

/// The stable device UID. Must not change across launches — the
/// system keeps per-device settings keyed on it.
pub const DEVICE_UID: &str = "com.tympan.aspl.Gain";

/// Sample rate the virtual device runs at.
pub const SAMPLE_RATE: f64 = 48_000.0;

/// Channel count for both the input and the output stream.
pub const CHANNELS: u32 = 2;

/// The linear gain applied to audio entering the device — `0.5`,
/// i.e. −6 dB. A real driver would make this configurable; here it
/// is a constant the [`Gain::new`] constructor copies into the
/// instance.
pub const GAIN: f32 = 0.5;

/// The CFPlugIn factory UUID for this driver's bundle — the key of
/// the `Info.plist`'s `CFPlugInFactories` dictionary. Unique per
/// driver.
pub const FACTORY_UUID: &str = "CC15176B-089F-4DEC-9A7A-4F7F806C961B";

/// A fixed-gain virtual loopback device.
///
/// The struct carries its processing configuration — the gain — as
/// a field, initialised in [`Driver::new`]. A driver with evolving
/// state (a filter's memory, a delay line) would carry that here
/// too; see the `lowpass` example.
pub struct Gain {
    /// The linear gain applied on the `WriteMix` operation.
    gain: f32,
}

impl Gain {
    /// The [`BundleConfig`] describing this driver's `.driver`
    /// bundle. The committed `Info.plist` is exactly
    /// [`generate`](tympan_aspl::bundle::plist::generate)`(&Gain::bundle_config())`
    /// — the `committed_info_plist_matches_the_generator` test
    /// enforces it.
    #[must_use]
    pub const fn bundle_config() -> BundleConfig {
        BundleConfig::new(DEVICE_UID, FACTORY_UUID, "TympanAsplDriverFactory")
            .with_bundle_name("Gain")
            .with_executable("Gain")
            .with_version("0.1.0")
    }
}

impl Driver for Gain {
    const NAME: &'static str = "Tympan Gain";
    const MANUFACTURER: &'static str = "tympan-aspl";
    const VERSION: &'static str = "0.1.0";

    fn new() -> Self {
        Self { gain: GAIN }
    }

    fn device(&self) -> DeviceSpec {
        let format = StreamFormat::float32(SAMPLE_RATE, CHANNELS);
        DeviceSpec::new(DEVICE_UID, "Gain", Self::MANUFACTURER)
            .with_sample_rate(SAMPLE_RATE)
            .with_input(StreamSpec::input(format))
            .with_output(StreamSpec::output(format))
    }

    fn process_io(&mut self, _rt: &RealtimeContext, buffer: &mut IoBuffer<'_>) {
        let n = buffer.output.len().min(buffer.input.len());
        match buffer.operation {
            // `WriteMix` — audio entering the device. Scale it; this
            // is the one operation where the gain is applied.
            IoOperation::WRITE_MIX => {
                for i in 0..n {
                    buffer.output[i] = buffer.input[i] * self.gain;
                }
            }
            // `ReadInput` (or anything else) — audio leaving the
            // device. Pass it through untouched so the gain is
            // applied exactly once across the loopback.
            _ => buffer.output[..n].copy_from_slice(&buffer.input[..n]),
        }
        // Pad a longer output with silence rather than leaving its
        // tail undefined.
        buffer.output[n..].fill(0.0);
    }
}

// Emit the `TympanAsplDriverFactory` CFPlugIn factory entry point —
// the symbol `coreaudiod` resolves from the bundle's `Info.plist`.
tympan_aspl::plugin_entry!(Gain);

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
        let driver = Gain::new();
        let spec = driver.device();
        assert_eq!(spec.uid(), DEVICE_UID);
        assert_eq!(spec.sample_rate(), SAMPLE_RATE);
        assert!(spec.is_loopback());
        assert_eq!(spec.input().unwrap().channels(), CHANNELS);
        assert_eq!(spec.output().unwrap().channels(), CHANNELS);
    }

    #[test]
    fn write_mix_scales_by_the_gain() {
        let mut driver = Gain::new();
        let input = [0.2_f32, -0.4, 1.0, -1.0];
        let mut output = [0.0_f32; 4];
        driver.process_io(
            &rt(),
            &mut buffer(IoOperation::WRITE_MIX, &input, &mut output),
        );
        assert_eq!(output, [0.1, -0.2, 0.5, -0.5]);
    }

    #[test]
    fn read_input_passes_through_unscaled() {
        let mut driver = Gain::new();
        let input = [0.2_f32, -0.4, 1.0, -1.0];
        let mut output = [0.0_f32; 4];
        driver.process_io(
            &rt(),
            &mut buffer(IoOperation::READ_INPUT, &input, &mut output),
        );
        // Pass-through: the gain is applied on `WriteMix`, not here,
        // so a sample looping through is scaled exactly once.
        assert_eq!(output, input);
    }

    #[test]
    fn process_io_pads_a_longer_output_with_silence() {
        let mut driver = Gain::new();
        let input = [1.0_f32, 1.0];
        let mut output = [9.0_f32; 4];
        driver.process_io(
            &rt(),
            &mut buffer(IoOperation::WRITE_MIX, &input, &mut output),
        );
        assert_eq!(output, [0.5, 0.5, 0.0, 0.0]);
    }

    #[test]
    fn lifecycle_runs_through_a_driver_instance() {
        let driver = DriverInstance::<Gain>::new();
        driver.initialize().unwrap();
        driver.start_io().unwrap();
        let input = [0.5_f32, 0.5];
        let mut output = [0.0_f32; 2];
        driver
            .process_io(
                &rt(),
                &mut buffer(IoOperation::WRITE_MIX, &input, &mut output),
            )
            .unwrap();
        assert_eq!(output, [0.25, 0.25]);
        driver.stop_io().unwrap();
    }

    #[test]
    fn identity_constants_are_wired_through() {
        let info = DriverInstance::<Gain>::new().info();
        assert_eq!(info.name, "Tympan Gain");
        assert_eq!(info.manufacturer, "tympan-aspl");
        assert_eq!(info.version, "0.1.0");
    }

    #[test]
    fn committed_info_plist_matches_the_generator() {
        // The committed `Info.plist` must stay byte-identical to
        // what `bundle::plist::generate` emits for this driver.
        assert_eq!(
            generate(&Gain::bundle_config()),
            include_str!("../Info.plist")
        );
    }
}
