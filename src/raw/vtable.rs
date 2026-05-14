//! The framework's `AudioServerPlugInDriverInterface` vtable.
//!
//! [`driver_interface`] returns a reference to the one `'static`
//! [`AudioServerPlugInDriverInterface`] the framework hands every
//! plug-in instance. Its slots point at the `extern "C"` entry
//! points in [`crate::raw::entry`]; the factory
//! ([`crate::raw::driver_factory_dispatch`]) stores a pointer to
//! this table as the first word of every [`DriverObject`], which is
//! what makes a `*mut DriverObject` a valid
//! [`AudioServerPlugInDriverRef`].
//!
//! ## Populated slots
//!
//! This table wires the `IUnknown` preamble, `Initialize`, the
//! device-client hooks, the full property protocol, and `StartIO` /
//! `StopIO`. The remaining slots — `CreateDevice` / `DestroyDevice`
//! (the framework's device is static, not HAL-created), the
//! configuration-change pair, and the five IO-cycle callbacks — are
//! `None` for now; the IO-cycle slots are filled when the realtime
//! IO path lands.
//!
//! Building the table needs no FFI, so its shape is unit-tested on
//! any host.
//!
//! [`DriverObject`]: crate::raw::runtime::DriverObject
//! [`AudioServerPlugInDriverRef`]: crate::raw::abi::AudioServerPlugInDriverRef

use crate::raw::abi::AudioServerPlugInDriverInterface;
use crate::raw::entry;

/// A `Sync` wrapper so the vtable can live in a `static`.
struct StaticInterface(AudioServerPlugInDriverInterface);

// Safety: the only non-`Sync` field of
// `AudioServerPlugInDriverInterface` is `_reserved`, a
// `*const c_void` the framework holds as null and never mutates or
// dereferences. Every other field is an `Option<extern "C" fn>`,
// which is `Sync`. Sharing the table across threads is therefore
// sound — which is exactly what a `static` requires.
unsafe impl Sync for StaticInterface {}

/// The single `'static` driver vtable, shared by every plug-in
/// instance the framework creates.
static DRIVER_INTERFACE: StaticInterface = StaticInterface(AudioServerPlugInDriverInterface {
    // `IUNKNOWN_C_GUTS`: the reserved slot is a null data pointer;
    // the three IUnknown methods follow.
    _reserved: core::ptr::null(),
    QueryInterface: Some(entry::query_interface),
    AddRef: Some(entry::add_ref),
    Release: Some(entry::release),

    Initialize: Some(entry::initialize),
    // The framework exposes a single static device declared by the
    // driver, so it never creates or destroys devices on HAL
    // request.
    CreateDevice: None,
    DestroyDevice: None,
    AddDeviceClient: Some(entry::add_device_client),
    RemoveDeviceClient: Some(entry::remove_device_client),
    // Deferred configuration changes are not yet modelled.
    PerformDeviceConfigurationChange: None,
    AbortDeviceConfigurationChange: None,

    HasProperty: Some(entry::has_property),
    IsPropertySettable: Some(entry::is_property_settable),
    GetPropertyDataSize: Some(entry::get_property_data_size),
    GetPropertyData: Some(entry::get_property_data),
    SetPropertyData: Some(entry::set_property_data),

    StartIO: Some(entry::start_io),
    StopIO: Some(entry::stop_io),
    // The realtime IO-cycle callbacks land with the IO path.
    GetZeroTimeStamp: None,
    WillDoIOOperation: None,
    BeginIOOperation: None,
    DoIOOperation: None,
    EndIOOperation: None,
});

/// A reference to the framework's `'static` driver vtable.
///
/// The factory stores this pointer as the first word of every
/// [`DriverObject`](crate::raw::runtime::DriverObject); the HAL then
/// calls through it.
#[must_use]
pub fn driver_interface() -> &'static AudioServerPlugInDriverInterface {
    &DRIVER_INTERFACE.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iunknown_preamble_is_wired() {
        let vtable = driver_interface();
        assert!(vtable._reserved.is_null());
        assert!(vtable.QueryInterface.is_some());
        assert!(vtable.AddRef.is_some());
        assert!(vtable.Release.is_some());
    }

    #[test]
    fn lifecycle_and_property_slots_are_wired() {
        let vtable = driver_interface();
        assert!(vtable.Initialize.is_some());
        assert!(vtable.AddDeviceClient.is_some());
        assert!(vtable.RemoveDeviceClient.is_some());
        assert!(vtable.HasProperty.is_some());
        assert!(vtable.IsPropertySettable.is_some());
        assert!(vtable.GetPropertyDataSize.is_some());
        assert!(vtable.GetPropertyData.is_some());
        assert!(vtable.SetPropertyData.is_some());
        assert!(vtable.StartIO.is_some());
        assert!(vtable.StopIO.is_some());
    }

    #[test]
    fn deferred_slots_are_none() {
        let vtable = driver_interface();
        // Static-device plug-in: no HAL-driven device lifecycle.
        assert!(vtable.CreateDevice.is_none());
        assert!(vtable.DestroyDevice.is_none());
        assert!(vtable.PerformDeviceConfigurationChange.is_none());
        assert!(vtable.AbortDeviceConfigurationChange.is_none());
        // The IO-cycle callbacks land with the realtime IO path.
        assert!(vtable.GetZeroTimeStamp.is_none());
        assert!(vtable.WillDoIOOperation.is_none());
        assert!(vtable.BeginIOOperation.is_none());
        assert!(vtable.DoIOOperation.is_none());
        assert!(vtable.EndIOOperation.is_none());
    }

    #[test]
    fn the_table_is_a_single_shared_instance() {
        // Two calls hand back the same `'static` table.
        assert!(core::ptr::eq(driver_interface(), driver_interface()));
    }
}
