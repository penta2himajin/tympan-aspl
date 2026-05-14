//! The `extern "C"` entry points `coreaudiod` calls through the
//! driver vtable.
//!
//! Each function here matches one slot of
//! [`AudioServerPlugInDriverInterface`]. The HAL calls them across
//! the C ABI; they recover the [`DriverObject`] from the ref the HAL
//! passes, translate the raw arguments through [`crate::raw::marshal`],
//! drive the cross-platform [`crate::dispatch`] / [`AnyDriver`](crate::driver::AnyDriver)
//! layers, and marshal the result back into the HAL's buffers.
//!
//! Although the functions are `extern "C"`, nothing in this module
//! is macOS-specific: an `extern "C"` function is ordinary Rust that
//! merely *uses the C calling convention*, and every argument type
//! is a plain pointer or scalar. So the entry points compile and are
//! unit-tested on any host — a test constructs a [`DriverObject`],
//! casts a pointer to it into an [`AudioServerPlugInDriverRef`], and
//! calls the entry points directly. The genuinely macOS-only pieces
//! — the `#[no_mangle]` factory symbol, the `CFStringRef`
//! construction for text properties, and the framework linkage —
//! live elsewhere (a later PR); see `write_cfstring`.
//!
//! ## Panic safety
//!
//! Entry points that call into user [`AnyDriver`](crate::driver::AnyDriver) code
//! ([`initialize`], [`start_io`], [`stop_io`]) wrap the call in
//! [`std::panic::catch_unwind`]: a panic must never unwind across
//! the C ABI. A caught panic is reported to the HAL as
//! [`OsStatus::UNSPECIFIED`].
//!
//! [`AudioServerPlugInDriverInterface`]: crate::raw::abi::AudioServerPlugInDriverInterface
//! [`AudioServerPlugInDriverRef`]: crate::raw::abi::AudioServerPlugInDriverRef

use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::dispatch;
use crate::error::OsStatus;
use crate::object::AudioObjectId;
use crate::raw::abi::{
    AudioObjectID, AudioObjectPropertyAddress, AudioServerPlugInDriverRef,
    AudioServerPlugInHostRef, Boolean, CFUUIDBytes, OSStatus, UInt32,
};
use crate::raw::marshal;
use crate::raw::platform;
use crate::raw::runtime::DriverObject;

/// `IUnknownUUID` — `{00000000-0000-0000-C000-000000000046}`. The
/// base COM interface every CFPlugIn interface also satisfies.
const IUNKNOWN_UUID: CFUUIDBytes = CFUUIDBytes {
    byte0: 0x00,
    byte1: 0x00,
    byte2: 0x00,
    byte3: 0x00,
    byte4: 0x00,
    byte5: 0x00,
    byte6: 0x00,
    byte7: 0x00,
    byte8: 0xC0,
    byte9: 0x00,
    byte10: 0x00,
    byte11: 0x00,
    byte12: 0x00,
    byte13: 0x00,
    byte14: 0x00,
    byte15: 0x46,
};

/// `kAudioServerPlugInDriverInterfaceUUID` —
/// `{EEA5773D-CC43-49F1-8E00-8F96E7D23B17}`, from
/// `<CoreAudio/AudioServerPlugIn.h>`. The interface `coreaudiod`
/// asks a plug-in's factory object for.
const ASPL_DRIVER_INTERFACE_UUID: CFUUIDBytes = CFUUIDBytes {
    byte0: 0xEE,
    byte1: 0xA5,
    byte2: 0x77,
    byte3: 0x3D,
    byte4: 0xCC,
    byte5: 0x43,
    byte6: 0x49,
    byte7: 0xF1,
    byte8: 0x8E,
    byte9: 0x00,
    byte10: 0x8F,
    byte11: 0x96,
    byte12: 0xE7,
    byte13: 0xD2,
    byte14: 0x3B,
    byte15: 0x17,
};

/// `E_NOINTERFACE` — the COM `HRESULT` for "this object does not
/// implement the requested interface", returned by
/// [`query_interface`].
const E_NOINTERFACE: OSStatus = 0x8000_0004_u32 as OSStatus;

/// Write a Rust string into a HAL property buffer as a
/// `CFStringRef`.
///
/// # Status
///
/// Stub. A text property crosses the ABI as a `CFStringRef` — a
/// CoreFoundation object pointer — and constructing one needs
/// CoreFoundation. The macOS implementation lands in a follow-up PR;
/// until then this returns [`OsStatus::UNSPECIFIED`], so the
/// text-valued properties (plug-in / device name, manufacturer,
/// device UID) are not yet served. Every other property value
/// variant is fully wired through [`get_property_data`].
fn write_cfstring(
    _text: &str,
    _out_data: *mut c_void,
    _capacity: usize,
) -> Result<usize, OsStatus> {
    Err(OsStatus::UNSPECIFIED)
}

/// Run `body`, catching any panic so it cannot unwind across the C
/// ABI. A caught panic becomes [`OsStatus::UNSPECIFIED`].
fn guard<F>(body: F) -> OsStatus
where
    F: FnOnce() -> OsStatus,
{
    catch_unwind(AssertUnwindSafe(body)).unwrap_or(OsStatus::UNSPECIFIED)
}

/// Borrow a HAL output buffer as a `&mut [u8]`.
///
/// # Safety
///
/// `ptr` must be null or point to at least `len` writable bytes;
/// the HAL upholds this for every `GetPropertyData` call. A null
/// pointer or zero length yields an empty slice.
unsafe fn out_slice<'a>(ptr: *mut c_void, len: UInt32) -> &'a mut [u8] {
    if ptr.is_null() || len == 0 {
        return &mut [];
    }
    // Safety: forwarded from the caller's contract.
    unsafe { core::slice::from_raw_parts_mut(ptr.cast::<u8>(), len as usize) }
}

/// Borrow a HAL input buffer as a `&[u8]`.
///
/// # Safety
///
/// `ptr` must be null or point to at least `len` readable bytes;
/// the HAL upholds this for every `SetPropertyData` call. A null
/// pointer or zero length yields an empty slice.
unsafe fn in_slice<'a>(ptr: *const c_void, len: UInt32) -> &'a [u8] {
    if ptr.is_null() || len == 0 {
        return &[];
    }
    // Safety: forwarded from the caller's contract.
    unsafe { core::slice::from_raw_parts(ptr.cast::<u8>(), len as usize) }
}

/// `IUnknown::QueryInterface` — slot 2 of the vtable.
///
/// The HAL asks the factory object whether it implements the
/// AudioServerPlugin driver interface (or the base `IUnknown`). For
/// either requested UUID this hands back the same object pointer,
/// `AddRef`s it, and returns `0` (`noErr`); for anything else it
/// nulls the out-pointer and returns `E_NOINTERFACE`.
///
/// # Safety
///
/// Called by the HAL across the C ABI. `this` must be a live
/// driver-object pointer (or null); `out_interface` must be a valid
/// writable `*mut *mut c_void` (or null).
pub unsafe extern "C" fn query_interface(
    this: *mut c_void,
    in_uuid_bytes: CFUUIDBytes,
    out_interface: *mut *mut c_void,
) -> OSStatus {
    // Safety: the HAL passes the factory object as `this`.
    let Some(object) = (unsafe { DriverObject::from_ref(this.cast()) }) else {
        return OsStatus::BAD_OBJECT.as_i32();
    };
    if in_uuid_bytes != IUNKNOWN_UUID && in_uuid_bytes != ASPL_DRIVER_INTERFACE_UUID {
        if !out_interface.is_null() {
            // Safety: caller guarantees `out_interface` is writable.
            unsafe { *out_interface = core::ptr::null_mut() };
        }
        return E_NOINTERFACE;
    }
    object.refcount().add_ref();
    if !out_interface.is_null() {
        // Safety: caller guarantees `out_interface` is writable.
        unsafe { *out_interface = this };
    }
    OsStatus::OK.as_i32()
}

/// `IUnknown::AddRef` — slot 3 of the vtable. Increments the
/// CFPlugIn reference count and returns the new value.
///
/// # Safety
///
/// Called by the HAL across the C ABI. `this` must be a live
/// driver-object pointer (or null).
pub unsafe extern "C" fn add_ref(this: *mut c_void) -> u32 {
    // Safety: the HAL passes a live driver-object pointer.
    match unsafe { DriverObject::from_ref(this.cast()) } {
        Some(object) => object.refcount().add_ref(),
        None => 0,
    }
}

/// `IUnknown::Release` — slot 4 of the vtable. Decrements the
/// CFPlugIn reference count, returns the new value, and frees the
/// driver object once the count reaches zero.
///
/// # Safety
///
/// Called by the HAL across the C ABI. `this` must be a live
/// driver-object pointer the framework's factory produced (or
/// null). After the call that drops the count to zero the HAL must
/// not touch `this` again — this function frees it.
pub unsafe extern "C" fn release(this: *mut c_void) -> u32 {
    // Safety: the HAL passes a live driver-object pointer.
    let Some(object) = (unsafe { DriverObject::from_ref(this.cast()) }) else {
        return 0;
    };
    let remaining = object.refcount().release();
    if remaining == 0 {
        // The HAL has dropped its last reference; we own the object
        // and reclaim the `Box` the factory leaked.
        // Safety: `this` came from `Box::into_raw` in the factory,
        // and the zero refcount means no other reference survives.
        drop(unsafe { Box::from_raw(this.cast::<DriverObject>()) });
    }
    remaining
}

/// `Initialize` — hands the plug-in the host interface and runs the
/// user driver's `initialize`.
///
/// # Safety
///
/// Called by the HAL across the C ABI. `driver` must be a live
/// driver ref; `host` is the host interface pointer (may be null).
pub unsafe extern "C" fn initialize(
    driver: AudioServerPlugInDriverRef,
    host: AudioServerPlugInHostRef,
) -> OSStatus {
    // Safety: the HAL passes a live driver ref.
    let Some(object) = (unsafe { DriverObject::from_ref(driver) }) else {
        return OsStatus::BAD_OBJECT.as_i32();
    };
    let runtime = object.runtime();
    runtime.set_host(host);
    guard(|| OsStatus::from(runtime.driver().initialize())).as_i32()
}

/// `StartIO` — runs the user driver's `start_io`, anchors the
/// device clock, and marks the device running.
///
/// # Safety
///
/// Called by the HAL across the C ABI. `driver` must be a live
/// driver ref.
pub unsafe extern "C" fn start_io(
    driver: AudioServerPlugInDriverRef,
    _device_id: AudioObjectID,
    _client_id: UInt32,
) -> OSStatus {
    // Safety: the HAL passes a live driver ref.
    let Some(object) = (unsafe { DriverObject::from_ref(driver) }) else {
        return OsStatus::BAD_OBJECT.as_i32();
    };
    let runtime = object.runtime();
    let status = guard(|| OsStatus::from(runtime.driver().start_io()));
    if status.is_ok() {
        runtime.state().running = true;
        // Anchor the zero-timestamp clock at the moment IO starts,
        // so `GetZeroTimeStamp` reports a timeline rooted here.
        runtime.clock().start(platform::host_time_now());
    }
    status.as_i32()
}

/// `StopIO` — runs the user driver's `stop_io`, halts the device
/// clock, and clears the running flag.
///
/// # Safety
///
/// Called by the HAL across the C ABI. `driver` must be a live
/// driver ref.
pub unsafe extern "C" fn stop_io(
    driver: AudioServerPlugInDriverRef,
    _device_id: AudioObjectID,
    _client_id: UInt32,
) -> OSStatus {
    // Safety: the HAL passes a live driver ref.
    let Some(object) = (unsafe { DriverObject::from_ref(driver) }) else {
        return OsStatus::BAD_OBJECT.as_i32();
    };
    let runtime = object.runtime();
    let status = guard(|| OsStatus::from(runtime.driver().stop_io()));
    if status.is_ok() {
        runtime.state().running = false;
        runtime.clock().stop();
    }
    status.as_i32()
}

/// `GetZeroTimeStamp` — report the device's clock anchor for the
/// current IO cycle.
///
/// Reads the host clock, asks the device's [`DeviceClock`] for the
/// most recent zero-timestamp-period boundary, and writes the
/// sample-time / host-time / seed triple into the HAL's out
/// pointers.
///
/// [`DeviceClock`]: crate::raw::clock::DeviceClock
///
/// # Safety
///
/// Called by the HAL across the C ABI. `driver` must be a live
/// driver ref; `out_sample_time`, `out_host_time`, and `out_seed`
/// must each be a valid writable pointer (or null).
pub unsafe extern "C" fn get_zero_time_stamp(
    driver: AudioServerPlugInDriverRef,
    _device_id: AudioObjectID,
    _client_id: UInt32,
    out_sample_time: *mut crate::raw::abi::Float64,
    out_host_time: *mut crate::raw::abi::UInt64,
    out_seed: *mut crate::raw::abi::UInt64,
) -> OSStatus {
    // Safety: the HAL passes a live driver ref.
    let Some(object) = (unsafe { DriverObject::from_ref(driver) }) else {
        return OsStatus::BAD_OBJECT.as_i32();
    };
    let runtime = object.runtime();
    // The zero-timestamp period is the device's nominal sample rate
    // worth of frames — a one-second ring (see `dispatch`).
    let period_frames = runtime.objects().spec().sample_rate();
    let sample_rate = runtime.state().sample_rate;
    let host_ticks_per_period = platform::host_ticks_per_period(period_frames, sample_rate);
    let now = platform::host_time_now();

    let Some(timestamp) = runtime
        .clock()
        .zero_timestamp(now, host_ticks_per_period, period_frames)
    else {
        // `GetZeroTimeStamp` is only legal while the device is
        // running; a stopped clock means the HAL called it out of
        // sequence.
        return OsStatus::NOT_RUNNING.as_i32();
    };

    if !out_sample_time.is_null() {
        // Safety: caller guarantees the pointer is writable.
        unsafe { *out_sample_time = timestamp.sample_time };
    }
    if !out_host_time.is_null() {
        // Safety: caller guarantees the pointer is writable.
        unsafe { *out_host_time = timestamp.host_time };
    }
    if !out_seed.is_null() {
        // Safety: caller guarantees the pointer is writable.
        unsafe { *out_seed = timestamp.seed };
    }
    OsStatus::OK.as_i32()
}

/// `WillDoIOOperation` — tell the HAL whether the framework handles
/// `operation_id`.
///
/// The framework does not yet move audio: the realtime data path
/// (`DoIOOperation`) lands in a follow-up PR. Until then this
/// answers "no" to every operation, so the HAL drives the device's
/// clock without expecting the plug-in to read or write samples —
/// the device runs and produces silence.
///
/// # Safety
///
/// Called by the HAL across the C ABI. `driver` must be a live
/// driver ref; `out_will_do` and `out_will_do_in_place` must each
/// be a valid writable `*mut Boolean` (or null).
pub unsafe extern "C" fn will_do_io_operation(
    driver: AudioServerPlugInDriverRef,
    _device_id: AudioObjectID,
    _client_id: UInt32,
    _operation_id: UInt32,
    out_will_do: *mut Boolean,
    out_will_do_in_place: *mut Boolean,
) -> OSStatus {
    // Safety: the HAL passes a live driver ref.
    let Some(_object) = (unsafe { DriverObject::from_ref(driver) }) else {
        return OsStatus::BAD_OBJECT.as_i32();
    };
    if !out_will_do.is_null() {
        // Safety: caller guarantees the pointer is writable.
        unsafe { *out_will_do = 0 };
    }
    if !out_will_do_in_place.is_null() {
        // Safety: caller guarantees the pointer is writable.
        unsafe { *out_will_do_in_place = 1 };
    }
    OsStatus::OK.as_i32()
}

/// `BeginIOOperation` — the HAL is about to run one IO operation.
///
/// A no-op for the framework's virtual device: there is no
/// per-operation setup to do. Validates the driver ref and returns
/// success.
///
/// # Safety
///
/// Called by the HAL across the C ABI. `driver` must be a live
/// driver ref; `io_cycle_info` may be null.
pub unsafe extern "C" fn begin_io_operation(
    driver: AudioServerPlugInDriverRef,
    _device_id: AudioObjectID,
    _client_id: UInt32,
    _operation_id: UInt32,
    _io_buffer_frame_size: UInt32,
    _io_cycle_info: *const crate::raw::abi::AudioServerPlugInIOCycleInfo,
) -> OSStatus {
    // Safety: the HAL passes a live driver ref.
    match unsafe { DriverObject::from_ref(driver) } {
        Some(_) => OsStatus::OK.as_i32(),
        None => OsStatus::BAD_OBJECT.as_i32(),
    }
}

/// `EndIOOperation` — the HAL has finished one IO operation.
///
/// The counterpart of [`begin_io_operation`]; also a no-op for the
/// virtual device.
///
/// # Safety
///
/// Called by the HAL across the C ABI. `driver` must be a live
/// driver ref; `io_cycle_info` may be null.
pub unsafe extern "C" fn end_io_operation(
    driver: AudioServerPlugInDriverRef,
    _device_id: AudioObjectID,
    _client_id: UInt32,
    _operation_id: UInt32,
    _io_buffer_frame_size: UInt32,
    _io_cycle_info: *const crate::raw::abi::AudioServerPlugInIOCycleInfo,
) -> OSStatus {
    // Safety: the HAL passes a live driver ref.
    match unsafe { DriverObject::from_ref(driver) } {
        Some(_) => OsStatus::OK.as_i32(),
        None => OsStatus::BAD_OBJECT.as_i32(),
    }
}

/// `AddDeviceClient` — a process opened one of the plug-in's
/// devices. The framework keeps no per-client state yet, so this
/// simply succeeds.
///
/// # Safety
///
/// Called by the HAL across the C ABI. `driver` must be a live
/// driver ref.
pub unsafe extern "C" fn add_device_client(
    driver: AudioServerPlugInDriverRef,
    _device_id: AudioObjectID,
    _client_info: *const crate::raw::abi::AudioServerPlugInClientInfo,
) -> OSStatus {
    // Safety: the HAL passes a live driver ref.
    match unsafe { DriverObject::from_ref(driver) } {
        Some(_) => OsStatus::OK.as_i32(),
        None => OsStatus::BAD_OBJECT.as_i32(),
    }
}

/// `RemoveDeviceClient` — a process closed one of the plug-in's
/// devices. The inverse of [`add_device_client`]; also a no-op
/// while the framework keeps no per-client state.
///
/// # Safety
///
/// Called by the HAL across the C ABI. `driver` must be a live
/// driver ref.
pub unsafe extern "C" fn remove_device_client(
    driver: AudioServerPlugInDriverRef,
    _device_id: AudioObjectID,
    _client_info: *const crate::raw::abi::AudioServerPlugInClientInfo,
) -> OSStatus {
    // Safety: the HAL passes a live driver ref.
    match unsafe { DriverObject::from_ref(driver) } {
        Some(_) => OsStatus::OK.as_i32(),
        None => OsStatus::BAD_OBJECT.as_i32(),
    }
}

/// `HasProperty` — does `object_id` have the addressed property?
///
/// # Safety
///
/// Called by the HAL across the C ABI. `driver` must be a live
/// driver ref; `address` must point to a valid
/// [`AudioObjectPropertyAddress`].
pub unsafe extern "C" fn has_property(
    driver: AudioServerPlugInDriverRef,
    object_id: AudioObjectID,
    _client_pid: crate::raw::abi::pid_t,
    address: *const AudioObjectPropertyAddress,
) -> Boolean {
    // Safety: the HAL passes a live driver ref and address pointer.
    let (Some(object), Some(raw_address)) =
        (unsafe { (DriverObject::from_ref(driver), address.as_ref()) })
    else {
        return 0;
    };
    let runtime = object.runtime();
    let address = marshal::address_from_raw(raw_address);
    let has = dispatch::get_property_data(
        runtime.objects(),
        runtime.info(),
        &runtime.state(),
        AudioObjectId::from_u32(object_id),
        &address,
    )
    .is_ok();
    Boolean::from(has)
}

/// `IsPropertySettable` — can the addressed property be written?
///
/// # Safety
///
/// Called by the HAL across the C ABI. `driver` must be a live
/// driver ref; `address` must point to a valid
/// [`AudioObjectPropertyAddress`]; `out_is_settable` must be a
/// valid writable `*mut Boolean` (or null).
pub unsafe extern "C" fn is_property_settable(
    driver: AudioServerPlugInDriverRef,
    object_id: AudioObjectID,
    _client_pid: crate::raw::abi::pid_t,
    address: *const AudioObjectPropertyAddress,
    out_is_settable: *mut Boolean,
) -> OSStatus {
    // Safety: the HAL passes a live driver ref and address pointer.
    let (Some(object), Some(raw_address)) =
        (unsafe { (DriverObject::from_ref(driver), address.as_ref()) })
    else {
        return OsStatus::BAD_OBJECT.as_i32();
    };
    let runtime = object.runtime();
    let address = marshal::address_from_raw(raw_address);
    match dispatch::is_property_settable(
        runtime.objects(),
        runtime.info(),
        &runtime.state(),
        AudioObjectId::from_u32(object_id),
        &address,
    ) {
        Ok(settable) => {
            if !out_is_settable.is_null() {
                // Safety: caller guarantees the pointer is writable.
                unsafe { *out_is_settable = Boolean::from(settable) };
            }
            OsStatus::OK.as_i32()
        }
        Err(status) => status.as_i32(),
    }
}

/// `GetPropertyDataSize` — how many bytes does the addressed
/// property's value occupy?
///
/// # Safety
///
/// Called by the HAL across the C ABI. `driver` must be a live
/// driver ref; `address` must point to a valid
/// [`AudioObjectPropertyAddress`]; `out_data_size` must be a valid
/// writable `*mut UInt32` (or null).
pub unsafe extern "C" fn get_property_data_size(
    driver: AudioServerPlugInDriverRef,
    object_id: AudioObjectID,
    _client_pid: crate::raw::abi::pid_t,
    address: *const AudioObjectPropertyAddress,
    _qualifier_size: UInt32,
    _qualifier_data: *const c_void,
    out_data_size: *mut UInt32,
) -> OSStatus {
    // Safety: the HAL passes a live driver ref and address pointer.
    let (Some(object), Some(raw_address)) =
        (unsafe { (DriverObject::from_ref(driver), address.as_ref()) })
    else {
        return OsStatus::BAD_OBJECT.as_i32();
    };
    let runtime = object.runtime();
    let address = marshal::address_from_raw(raw_address);
    match dispatch::property_data_size(
        runtime.objects(),
        runtime.info(),
        &runtime.state(),
        AudioObjectId::from_u32(object_id),
        &address,
    ) {
        Ok(size) => {
            if !out_data_size.is_null() {
                // Safety: caller guarantees the pointer is writable.
                unsafe { *out_data_size = size as UInt32 };
            }
            OsStatus::OK.as_i32()
        }
        Err(status) => status.as_i32(),
    }
}

/// `GetPropertyData` — read the addressed property's value into the
/// HAL's buffer.
///
/// # Safety
///
/// Called by the HAL across the C ABI. `driver` must be a live
/// driver ref; `address` must point to a valid
/// [`AudioObjectPropertyAddress`]; `out_data` must point to at
/// least `data_size` writable bytes (or be null); `out_data_size`
/// must be a valid writable `*mut UInt32` (or null).
#[allow(clippy::too_many_arguments)] // one parameter per C ABI argument
pub unsafe extern "C" fn get_property_data(
    driver: AudioServerPlugInDriverRef,
    object_id: AudioObjectID,
    _client_pid: crate::raw::abi::pid_t,
    address: *const AudioObjectPropertyAddress,
    _qualifier_size: UInt32,
    _qualifier_data: *const c_void,
    data_size: UInt32,
    out_data_size: *mut UInt32,
    out_data: *mut c_void,
) -> OSStatus {
    // Safety: the HAL passes a live driver ref and address pointer.
    let (Some(object), Some(raw_address)) =
        (unsafe { (DriverObject::from_ref(driver), address.as_ref()) })
    else {
        return OsStatus::BAD_OBJECT.as_i32();
    };
    let runtime = object.runtime();
    let address = marshal::address_from_raw(raw_address);
    let value = match dispatch::get_property_data(
        runtime.objects(),
        runtime.info(),
        &runtime.state(),
        AudioObjectId::from_u32(object_id),
        &address,
    ) {
        Ok(value) => value,
        Err(status) => return status.as_i32(),
    };

    // A text value crosses the ABI as a `CFStringRef`; everything
    // else is plain bytes the marshaller writes directly.
    let written = if let crate::property::PropertyValue::Text(text) = &value {
        write_cfstring(text, out_data, data_size as usize)
    } else {
        // Safety: the HAL guarantees `out_data` covers `data_size`
        // writable bytes.
        let buffer = unsafe { out_slice(out_data, data_size) };
        marshal::write_property_value(&value, buffer)
    };

    match written {
        Ok(size) => {
            if !out_data_size.is_null() {
                // Safety: caller guarantees the pointer is writable.
                unsafe { *out_data_size = size as UInt32 };
            }
            OsStatus::OK.as_i32()
        }
        Err(status) => status.as_i32(),
    }
}

/// `SetPropertyData` — write the addressed property's value from the
/// HAL's buffer.
///
/// # Safety
///
/// Called by the HAL across the C ABI. `driver` must be a live
/// driver ref; `address` must point to a valid
/// [`AudioObjectPropertyAddress`]; `data` must point to at least
/// `data_size` readable bytes (or be null).
#[allow(clippy::too_many_arguments)] // one parameter per C ABI argument
pub unsafe extern "C" fn set_property_data(
    driver: AudioServerPlugInDriverRef,
    object_id: AudioObjectID,
    _client_pid: crate::raw::abi::pid_t,
    address: *const AudioObjectPropertyAddress,
    _qualifier_size: UInt32,
    _qualifier_data: *const c_void,
    data_size: UInt32,
    data: *const c_void,
) -> OSStatus {
    // Safety: the HAL passes a live driver ref and address pointer.
    let (Some(object), Some(raw_address)) =
        (unsafe { (DriverObject::from_ref(driver), address.as_ref()) })
    else {
        return OsStatus::BAD_OBJECT.as_i32();
    };
    let runtime = object.runtime();
    let address = marshal::address_from_raw(raw_address);
    // Safety: the HAL guarantees `data` covers `data_size` readable
    // bytes.
    let buffer = unsafe { in_slice(data, data_size) };
    let mut state = runtime.state();
    let result = dispatch::set_property_data(
        runtime.objects(),
        runtime.info(),
        &mut state,
        AudioObjectId::from_u32(object_id),
        &address,
        buffer,
    );
    OsStatus::from(result).as_i32()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::{AnyDriver, Driver, DriverInstance};
    use crate::property::{PropertySelector, PropertyValue};
    use crate::raw::runtime::DriverRuntime;
    use crate::{DeviceSpec, IoBuffer, RealtimeContext, StreamFormat, StreamSpec};
    use std::sync::Arc;

    struct Loopback;

    impl Driver for Loopback {
        const NAME: &'static str = "tympan-aspl entry fixture";
        const MANUFACTURER: &'static str = "tympan-aspl";
        const VERSION: &'static str = "0.0.0";

        fn new() -> Self {
            Self
        }

        fn device(&self) -> DeviceSpec {
            let format = StreamFormat::float32(48_000.0, 2);
            DeviceSpec::new("com.tympan.test.entry", "Entry Fixture", Self::MANUFACTURER)
                .with_input(StreamSpec::input(format))
                .with_output(StreamSpec::output(format))
        }

        fn process_io(&mut self, _rt: &RealtimeContext, _buffer: &mut IoBuffer<'_>) {}
    }

    /// A live driver object plus the raw ref the entry points take.
    /// The object is leaked via `Box::into_raw`; [`release`] frees
    /// it, mirroring the HAL's ownership.
    struct Fixture {
        driver_ref: AudioServerPlugInDriverRef,
    }

    impl Fixture {
        fn new() -> Self {
            let driver: Arc<dyn AnyDriver> = Arc::new(DriverInstance::<Loopback>::new());
            let object = Box::new(DriverObject::new(
                crate::raw::vtable::driver_interface(),
                DriverRuntime::new(driver),
            ));
            let raw = Box::into_raw(object);
            Self {
                driver_ref: raw.cast(),
            }
        }

        fn this(&self) -> *mut c_void {
            self.driver_ref.cast()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            // Mirror the HAL's final `Release`: drops the refcount
            // from 1 to 0 and frees the box.
            // Safety: the fixture owns the only reference.
            let remaining = unsafe { release(self.this()) };
            assert_eq!(remaining, 0, "fixture leaked a reference");
        }
    }

    fn global(selector: PropertySelector) -> AudioObjectPropertyAddress {
        marshal::address_to_raw(&crate::property::PropertyAddress::global(selector))
    }

    #[test]
    fn query_interface_accepts_the_driver_and_iunknown_uuids() {
        let fixture = Fixture::new();
        for uuid in [ASPL_DRIVER_INTERFACE_UUID, IUNKNOWN_UUID] {
            let mut out: *mut c_void = core::ptr::null_mut();
            // Safety: `this()` is a live driver object.
            let status = unsafe { query_interface(fixture.this(), uuid, &mut out) };
            assert_eq!(status, OsStatus::OK.as_i32());
            assert_eq!(out, fixture.this());
        }
        // Each successful QueryInterface AddRef'd; balance the count
        // back so the fixture's drop-time Release lands on zero.
        // Safety: `this()` is live; we AddRef'd twice above.
        unsafe {
            release(fixture.this());
            release(fixture.this());
        }
    }

    #[test]
    fn query_interface_rejects_an_unknown_uuid() {
        let fixture = Fixture::new();
        let bogus = CFUUIDBytes {
            byte0: 0xFF,
            ..CFUUIDBytes::default()
        };
        let mut out: *mut c_void = 0x1 as *mut c_void;
        // Safety: `this()` is a live driver object.
        let status = unsafe { query_interface(fixture.this(), bogus, &mut out) };
        assert_eq!(status, E_NOINTERFACE);
        assert!(out.is_null());
    }

    #[test]
    fn add_ref_and_release_track_the_count() {
        let fixture = Fixture::new();
        // Safety: `this()` is a live driver object.
        unsafe {
            assert_eq!(add_ref(fixture.this()), 2);
            assert_eq!(add_ref(fixture.this()), 3);
            assert_eq!(release(fixture.this()), 2);
            assert_eq!(release(fixture.this()), 1);
        }
        // Fixture drop performs the final release to zero.
    }

    #[test]
    fn null_driver_ref_is_handled() {
        let null = core::ptr::null_mut::<c_void>();
        // Safety: null is explicitly handled by every entry point.
        unsafe {
            assert_eq!(add_ref(null), 0);
            assert_eq!(release(null), 0);
            assert_eq!(
                initialize(null.cast(), core::ptr::null_mut()),
                OsStatus::BAD_OBJECT.as_i32()
            );
            let addr = global(PropertySelector::CLASS);
            assert_eq!(has_property(null.cast(), 1, 0, &addr), 0);
        }
    }

    #[test]
    fn initialize_start_stop_drive_the_lifecycle() {
        let fixture = Fixture::new();
        // Safety: `this()` is a live driver object.
        unsafe {
            assert_eq!(
                initialize(fixture.driver_ref, core::ptr::null_mut()),
                OsStatus::OK.as_i32()
            );
            let object = DriverObject::from_ref(fixture.driver_ref).unwrap();
            assert!(!object.runtime().state().running);

            assert_eq!(start_io(fixture.driver_ref, 2, 0), OsStatus::OK.as_i32());
            assert!(object.runtime().state().running);

            assert_eq!(stop_io(fixture.driver_ref, 2, 0), OsStatus::OK.as_i32());
            assert!(!object.runtime().state().running);
        }
    }

    #[test]
    fn initialize_records_the_host_pointer() {
        let fixture = Fixture::new();
        let fake_host = 0xCAFE_usize as AudioServerPlugInHostRef;
        // Safety: `this()` is a live driver object; the host pointer
        // is only stored, never dereferenced.
        unsafe {
            assert_eq!(
                initialize(fixture.driver_ref, fake_host),
                OsStatus::OK.as_i32()
            );
            let object = DriverObject::from_ref(fixture.driver_ref).unwrap();
            assert_eq!(object.runtime().host(), fake_host);
        }
    }

    #[test]
    fn has_property_distinguishes_known_and_unknown() {
        let fixture = Fixture::new();
        let known = global(PropertySelector::CLASS);
        let unknown = global(PropertySelector::new(*b"zzzz"));
        // Safety: `driver_ref` is live; the addresses are valid.
        unsafe {
            assert_eq!(has_property(fixture.driver_ref, 1, 0, &known), 1);
            assert_eq!(has_property(fixture.driver_ref, 1, 0, &unknown), 0);
            // An object id outside the tree.
            assert_eq!(has_property(fixture.driver_ref, 999, 0, &known), 0);
        }
    }

    #[test]
    fn is_property_settable_reports_the_sample_rate_as_writable() {
        let fixture = Fixture::new();
        let rate = global(PropertySelector::DEVICE_NOMINAL_SAMPLE_RATE);
        let uid = global(PropertySelector::DEVICE_UID);
        let mut settable: Boolean = 0xFF;
        // Safety: `driver_ref` is live; addresses and out-pointer are
        // valid.
        unsafe {
            // The device is object id 2.
            assert_eq!(
                is_property_settable(fixture.driver_ref, 2, 0, &rate, &mut settable),
                OsStatus::OK.as_i32()
            );
            assert_eq!(settable, 1);
            assert_eq!(
                is_property_settable(fixture.driver_ref, 2, 0, &uid, &mut settable),
                OsStatus::OK.as_i32()
            );
            assert_eq!(settable, 0);
        }
    }

    #[test]
    fn get_property_data_size_and_data_agree_for_a_scalar() {
        let fixture = Fixture::new();
        let rate = global(PropertySelector::DEVICE_NOMINAL_SAMPLE_RATE);
        let mut size: UInt32 = 0;
        let mut buffer = [0u8; 8];
        let mut written: UInt32 = 0;
        // Safety: `driver_ref` is live; the buffers are valid and
        // sized.
        unsafe {
            assert_eq!(
                get_property_data_size(
                    fixture.driver_ref,
                    2,
                    0,
                    &rate,
                    0,
                    core::ptr::null(),
                    &mut size,
                ),
                OsStatus::OK.as_i32()
            );
            assert_eq!(size, 8);

            assert_eq!(
                get_property_data(
                    fixture.driver_ref,
                    2,
                    0,
                    &rate,
                    0,
                    core::ptr::null(),
                    buffer.len() as UInt32,
                    &mut written,
                    buffer.as_mut_ptr().cast(),
                ),
                OsStatus::OK.as_i32()
            );
            assert_eq!(written, 8);
        }
        assert_eq!(f64::from_ne_bytes(buffer), 48_000.0);
    }

    #[test]
    fn get_property_data_reads_an_object_list() {
        let fixture = Fixture::new();
        let device_list = global(PropertySelector::PLUGIN_DEVICE_LIST);
        let mut buffer = [0u8; 4];
        let mut written: UInt32 = 0;
        // Safety: `driver_ref` is live; the buffer is valid and sized
        // for one AudioObjectID.
        unsafe {
            assert_eq!(
                get_property_data(
                    fixture.driver_ref,
                    1,
                    0,
                    &device_list,
                    0,
                    core::ptr::null(),
                    buffer.len() as UInt32,
                    &mut written,
                    buffer.as_mut_ptr().cast(),
                ),
                OsStatus::OK.as_i32()
            );
        }
        assert_eq!(written, 4);
        // The device is object id 2.
        assert_eq!(u32::from_ne_bytes(buffer), 2);
    }

    #[test]
    fn get_property_data_rejects_an_unknown_property() {
        let fixture = Fixture::new();
        let unknown = global(PropertySelector::new(*b"zzzz"));
        let mut buffer = [0u8; 8];
        let mut written: UInt32 = 0;
        // Safety: `driver_ref` is live; the buffer is valid.
        let status = unsafe {
            get_property_data(
                fixture.driver_ref,
                1,
                0,
                &unknown,
                0,
                core::ptr::null(),
                buffer.len() as UInt32,
                &mut written,
                buffer.as_mut_ptr().cast(),
            )
        };
        assert_eq!(status, OsStatus::UNKNOWN_PROPERTY.as_i32());
    }

    #[test]
    fn text_properties_are_not_yet_served() {
        // `write_cfstring` is a stub until the macOS CoreFoundation
        // layer lands; a text property therefore reports
        // `UNSPECIFIED` rather than a wrong value.
        let fixture = Fixture::new();
        let name = global(PropertySelector::NAME);
        let mut buffer = [0u8; 8];
        let mut written: UInt32 = 0;
        // Safety: `driver_ref` is live; the buffer is valid.
        let status = unsafe {
            get_property_data(
                fixture.driver_ref,
                1,
                0,
                &name,
                0,
                core::ptr::null(),
                buffer.len() as UInt32,
                &mut written,
                buffer.as_mut_ptr().cast(),
            )
        };
        assert_eq!(status, OsStatus::UNSPECIFIED.as_i32());

        // The dispatcher still *has* the property — only the
        // marshalling is pending.
        // Safety: `driver_ref` is the live object the fixture built.
        let object = unsafe { DriverObject::from_ref(fixture.driver_ref) }.unwrap();
        let runtime = object.runtime();
        let value = dispatch::get_property_data(
            runtime.objects(),
            runtime.info(),
            &runtime.state(),
            AudioObjectId::PLUGIN,
            &crate::property::PropertyAddress::global(PropertySelector::NAME),
        );
        assert!(matches!(value, Ok(PropertyValue::Text(_))));
    }

    #[test]
    fn set_property_data_writes_the_sample_rate() {
        let fixture = Fixture::new();
        let rate = global(PropertySelector::DEVICE_NOMINAL_SAMPLE_RATE);
        let payload = 48_000.0_f64.to_ne_bytes();
        // Safety: `driver_ref` is live; the payload is a valid
        // 8-byte buffer.
        let status = unsafe {
            set_property_data(
                fixture.driver_ref,
                2,
                0,
                &rate,
                0,
                core::ptr::null(),
                payload.len() as UInt32,
                payload.as_ptr().cast(),
            )
        };
        assert_eq!(status, OsStatus::OK.as_i32());

        // An unsupported rate is rejected.
        let payload = 12_345.0_f64.to_ne_bytes();
        // Safety: as above.
        let status = unsafe {
            set_property_data(
                fixture.driver_ref,
                2,
                0,
                &rate,
                0,
                core::ptr::null(),
                payload.len() as UInt32,
                payload.as_ptr().cast(),
            )
        };
        assert_eq!(status, OsStatus::UNSUPPORTED_FORMAT.as_i32());
    }

    #[test]
    fn add_and_remove_device_client_succeed() {
        let fixture = Fixture::new();
        // Safety: `driver_ref` is live; the client-info pointer is
        // not dereferenced by these no-op stubs.
        unsafe {
            assert_eq!(
                add_device_client(fixture.driver_ref, 2, core::ptr::null()),
                OsStatus::OK.as_i32()
            );
            assert_eq!(
                remove_device_client(fixture.driver_ref, 2, core::ptr::null()),
                OsStatus::OK.as_i32()
            );
        }
    }

    #[test]
    fn start_io_anchors_the_clock_and_stop_io_halts_it() {
        let fixture = Fixture::new();
        // Safety: `driver_ref` is the live object the fixture built.
        let object = unsafe { DriverObject::from_ref(fixture.driver_ref) }.unwrap();
        assert!(!object.runtime().clock().is_running());

        // Safety: `driver_ref` is live.
        unsafe {
            initialize(fixture.driver_ref, core::ptr::null_mut());
            assert_eq!(start_io(fixture.driver_ref, 2, 0), OsStatus::OK.as_i32());
        }
        assert!(object.runtime().clock().is_running());
        assert_eq!(object.runtime().clock().seed(), 1);

        // Safety: `driver_ref` is live.
        unsafe {
            assert_eq!(stop_io(fixture.driver_ref, 2, 0), OsStatus::OK.as_i32());
        }
        assert!(!object.runtime().clock().is_running());
    }

    #[test]
    fn get_zero_time_stamp_requires_a_running_device() {
        let fixture = Fixture::new();
        let mut sample_time = 0.0;
        let mut host_time = 0u64;
        let mut seed = 0u64;
        // Before `StartIO` the clock is stopped — out of sequence.
        // Safety: `driver_ref` is live; the out-pointers are valid.
        let status = unsafe {
            get_zero_time_stamp(
                fixture.driver_ref,
                2,
                0,
                &mut sample_time,
                &mut host_time,
                &mut seed,
            )
        };
        assert_eq!(status, OsStatus::NOT_RUNNING.as_i32());
    }

    #[test]
    fn get_zero_time_stamp_reports_a_timeline_once_running() {
        let fixture = Fixture::new();
        // Safety: `driver_ref` is live.
        unsafe {
            initialize(fixture.driver_ref, core::ptr::null_mut());
            start_io(fixture.driver_ref, 2, 0);
        }

        let mut sample_time = -1.0;
        let mut host_time = 0u64;
        let mut seed = 0u64;
        // Safety: `driver_ref` is live; the out-pointers are valid.
        let status = unsafe {
            get_zero_time_stamp(
                fixture.driver_ref,
                2,
                0,
                &mut sample_time,
                &mut host_time,
                &mut seed,
            )
        };
        assert_eq!(status, OsStatus::OK.as_i32());
        // The timeline starts at or after period zero, and the seed
        // is the one `StartIO` established.
        assert!(sample_time >= 0.0);
        assert_eq!(seed, 1);
    }

    #[test]
    fn get_zero_time_stamp_tolerates_null_out_pointers() {
        let fixture = Fixture::new();
        // Safety: `driver_ref` is live.
        unsafe {
            initialize(fixture.driver_ref, core::ptr::null_mut());
            start_io(fixture.driver_ref, 2, 0);
            // Every out-pointer null: the entry point must not
            // dereference them.
            assert_eq!(
                get_zero_time_stamp(
                    fixture.driver_ref,
                    2,
                    0,
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                ),
                OsStatus::OK.as_i32()
            );
        }
    }

    #[test]
    fn will_do_io_operation_declines_every_operation_for_now() {
        let fixture = Fixture::new();
        let mut will_do: Boolean = 0xFF;
        let mut will_do_in_place: Boolean = 0xFF;
        // Safety: `driver_ref` is live; the out-pointers are valid.
        let status = unsafe {
            will_do_io_operation(
                fixture.driver_ref,
                2,
                0,
                crate::io::IoOperation::PROCESS_OUTPUT.code().as_u32(),
                &mut will_do,
                &mut will_do_in_place,
            )
        };
        assert_eq!(status, OsStatus::OK.as_i32());
        // The data path is not wired yet — the framework handles no
        // operation.
        assert_eq!(will_do, 0);
    }

    #[test]
    fn begin_and_end_io_operation_succeed() {
        let fixture = Fixture::new();
        // Safety: `driver_ref` is live; the cycle-info pointer is
        // not dereferenced by these no-op brackets.
        unsafe {
            assert_eq!(
                begin_io_operation(fixture.driver_ref, 2, 0, 0, 512, core::ptr::null()),
                OsStatus::OK.as_i32()
            );
            assert_eq!(
                end_io_operation(fixture.driver_ref, 2, 0, 0, 512, core::ptr::null()),
                OsStatus::OK.as_i32()
            );
        }
    }

    #[test]
    fn io_timing_entry_points_reject_a_null_driver() {
        let null = core::ptr::null_mut::<c_void>().cast();
        let mut scratch = 0u64;
        let mut scratch_f = 0.0;
        let mut flag: Boolean = 0;
        // Safety: null is explicitly handled by every entry point.
        unsafe {
            assert_eq!(
                get_zero_time_stamp(null, 2, 0, &mut scratch_f, &mut scratch, &mut scratch),
                OsStatus::BAD_OBJECT.as_i32()
            );
            assert_eq!(
                will_do_io_operation(null, 2, 0, 0, &mut flag, &mut flag),
                OsStatus::BAD_OBJECT.as_i32()
            );
            assert_eq!(
                begin_io_operation(null, 2, 0, 0, 0, core::ptr::null()),
                OsStatus::BAD_OBJECT.as_i32()
            );
            assert_eq!(
                end_io_operation(null, 2, 0, 0, 0, core::ptr::null()),
                OsStatus::BAD_OBJECT.as_i32()
            );
        }
    }
}
