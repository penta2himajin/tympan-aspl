# minimal-loopback

The smallest interesting `tympan-aspl` driver: a stereo virtual
loopback device whose output is routed straight back to its input.
Anything an application plays into the device can be captured from
it — the macOS analogue of `tympan-apo`'s `passthrough` example.

## What it demonstrates

- Implementing the [`Driver`] trait: the three identity constants
  (`NAME`, `MANUFACTURER`, `VERSION`) and the `new` / `device` /
  `process_io` methods.
- Describing a loopback device with a `DeviceSpec` that carries both
  an input and an output `StreamSpec` in the canonical interleaved
  float32 format.
- A realtime-safe `process_io` body — a single `copy_from_slice`,
  with no allocation and no locks.

## Status

This crate is an `rlib` today. The cross-platform `Driver`
implementation in `src/lib.rs` builds and is unit-tested on every
host:

```bash
cargo test -p minimal-loopback
```

Producing the loadable `MinimalLoopback.driver` bundle needs the
`raw` FFI bridge, which lands in a follow-up PR per
[`docs/decisions/0001-ci-verification-strategy.md`](../../docs/decisions/0001-ci-verification-strategy.md).
When it does, two changes turn this crate into a bundle:

1. switch `crate-type` to `["cdylib"]` in `Cargo.toml`, and
2. add `tympan_aspl::plugin_entry!(MinimalLoopback);` at the crate
   root to emit the CFPlugIn factory symbol.

## The bundle layout

`Info.plist` in this directory already describes the intended
bundle. Tier 2 CI `plutil`-lints it, and it is exactly what
`tympan_aspl::bundle::plist::generate` emits for this driver's
[`BundleConfig`] — see the `print-info-plist` example at the
repository root.

```text
MinimalLoopback.driver/
└── Contents/
    ├── Info.plist          ← the file in this directory
    └── MacOS/
        └── MinimalLoopback ← the cdylib, once raw FFI lands
```

Once built, the bundle installs into the HAL plug-in directory:

```bash
sudo cp -R MinimalLoopback.driver /Library/Audio/Plug-Ins/HAL/
sudo killall coreaudiod
```

and the device appears in System Settings ▸ Sound.

[`Driver`]: https://docs.rs/tympan-aspl
[`BundleConfig`]: https://docs.rs/tympan-aspl
