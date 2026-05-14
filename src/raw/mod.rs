//! Low-level FFI to the Core Audio AudioServerPlugin C ABI.
//!
//! This module is the sole place that will link `CoreFoundation` and
//! `CoreAudio` and the sole owner of the CFPlugIn `IUnknown`-style
//! vtable bookkeeping required to expose Rust types as a plug-in
//! `coreaudiod` can load. It is gated on `cfg(target_os = "macos")`
//! because none of that machinery exists on other platforms — the
//! cross-platform layers (`realtime`, `driver`, `device`, `stream`,
//! `bundle`, …) are deliberately kept FFI-free so their invariants
//! can be unit-tested on any host.
//!
//! ## Status
//!
//! Stub. The cross-platform foundation lands first; the FFI bridge
//! is built on top of it in a follow-up PR. When it does, this
//! module gains the submodules sketched in `docs/architecture.md`:
//!
//! - `vtable` — CFPlugIn `IUnknown` vtable construction and the
//!   `AudioServerPlugInDriverInterface` function-pointer table.
//! - `selectors` — the raw property-selector / scope constants,
//!   cross-checked against `coreaudio-sys` with `static_assertions`.
//! - `host` — the `AudioServerPlugInHostInterface` side, i.e. the
//!   callbacks `coreaudiod` hands the plug-in.
//!
//! Users of `tympan-aspl` are not expected to touch this module; the
//! public API in the crate root wraps it. It is `pub` only for the
//! framework's own [`plugin_entry!`](crate::plugin_entry) macro and
//! for advanced users who need to bypass the higher-level
//! abstractions.

use std::os::raw::c_void;
use std::sync::Arc;

use crate::driver::AnyDriver;

/// Raw Core Audio object identifier — the C `AudioObjectID`.
///
/// The safe wrapper is [`AudioObjectId`](crate::object::AudioObjectId);
/// this alias exists for the FFI surface that will be built on top
/// of this module.
pub type RawObjectId = u32;

/// Raw Core Audio result code — the C `OSStatus`.
///
/// The safe wrapper is [`OsStatus`](crate::error::OsStatus).
pub type RawOsStatus = i32;

/// The CFPlugIn factory entry point, dispatched from the
/// [`plugin_entry!`](crate::plugin_entry) macro.
///
/// `coreaudiod` resolves the `#[no_mangle] extern "C"` factory
/// symbol the macro emits and calls it with a `CFAllocatorRef` and
/// the requested CFPlugIn type UUID; the macro forwards both, plus a
/// `create` constructor for the user's [`AnyDriver`], here.
///
/// # Status
///
/// Stub: returns a null interface pointer. The real implementation
/// builds the `AudioServerPlugInDriverInterface` vtable, wraps it in
/// the CFPlugIn `IUnknown` layout, and hands `coreaudiod` a live
/// `AudioServerPlugInDriverRef`. Until then the macro is exercised
/// for compilation only — `coreaudiod` will not load a plug-in
/// whose factory returns null, which the Tier 2/3 CI work tracks.
///
/// # Safety
///
/// Called by `coreaudiod` across the C ABI. `allocator` and
/// `requested_type_uuid` are the `CFAllocatorRef` and `CFUUIDRef`
/// the loader passes; both may be null. The current stub does not
/// dereference either, so any pointer value is sound for this
/// implementation — that contract tightens when the real FFI lands.
pub unsafe fn driver_factory_dispatch(
    allocator: *const c_void,
    requested_type_uuid: *const c_void,
    create: fn() -> Arc<dyn AnyDriver>,
) -> *mut c_void {
    // The HAL has not been given a real interface yet. We still
    // exercise the `create` constructor so the macro's wiring is
    // type-checked end to end and the user's `Driver::new` is
    // proven to compile through the framework's type-erased path.
    let _instance: Arc<dyn AnyDriver> = create();
    let _ = (allocator, requested_type_uuid);
    core::ptr::null_mut()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::{Driver, DriverInstance};
    use crate::{DeviceSpec, IoBuffer, RealtimeContext};

    struct NullDriver;

    impl Driver for NullDriver {
        const NAME: &'static str = "tympan-aspl raw stub driver";
        const MANUFACTURER: &'static str = "tympan-aspl";
        const VERSION: &'static str = "0.0.0";

        fn new() -> Self {
            Self
        }

        fn device(&self) -> DeviceSpec {
            DeviceSpec::new("com.tympan.test.raw", "Raw Stub", Self::MANUFACTURER)
        }

        fn process_io(&mut self, _rt: &RealtimeContext, _buffer: &mut IoBuffer<'_>) {}
    }

    fn create() -> Arc<dyn AnyDriver> {
        Arc::new(DriverInstance::<NullDriver>::new())
    }

    #[test]
    fn factory_dispatch_stub_returns_null_but_runs_create() {
        // Safety: the stub does not dereference either pointer, so
        // passing null for both is sound for this implementation.
        let ptr = unsafe { driver_factory_dispatch(core::ptr::null(), core::ptr::null(), create) };
        assert!(ptr.is_null());
    }

    #[test]
    fn raw_aliases_match_safe_wrapper_widths() {
        use core::mem::size_of;
        assert_eq!(size_of::<RawObjectId>(), size_of::<crate::AudioObjectId>());
        assert_eq!(size_of::<RawOsStatus>(), size_of::<crate::OsStatus>());
    }
}
