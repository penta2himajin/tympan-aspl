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

## Building

The crate builds two ways. As an `rlib`, its `Driver` implementation
is unit-tested on every host:

```bash
cargo test -p minimal-loopback
```

As a `cdylib`, it is the loadable plug-in binary —
`tympan_aspl::plugin_entry!` emits the `TympanAsplDriverFactory`
CFPlugIn entry point `coreaudiod` resolves:

```bash
cargo build --release -p minimal-loopback
# → target/release/libminimal_loopback.dylib
```

## The bundle layout

The loadable `MinimalLoopback.driver` bundle is the committed
`Info.plist` plus the built cdylib, in the CFBundle layout
`coreaudiod` expects:

```text
MinimalLoopback.driver/
└── Contents/
    ├── Info.plist          ← the file in this directory
    └── MacOS/
        └── MinimalLoopback ← the built cdylib
```

`Info.plist` is exactly what `tympan_aspl::bundle::plist::generate`
emits for this driver's [`BundleConfig`] — see the `print-info-plist`
example at the repository root. Tier 2 CI assembles the bundle,
`plutil`-lints the plist, `nm`-checks the factory symbol, and ad-hoc
code-signs it.

To assemble and install the bundle by hand:

```bash
BUNDLE=MinimalLoopback.driver
mkdir -p "$BUNDLE/Contents/MacOS"
cp examples/minimal-loopback/Info.plist "$BUNDLE/Contents/Info.plist"
cp target/release/libminimal_loopback.dylib "$BUNDLE/Contents/MacOS/MinimalLoopback"
codesign --force --sign - "$BUNDLE"

sudo cp -R "$BUNDLE" /Library/Audio/Plug-Ins/HAL/
sudo killall coreaudiod
```

and the device appears in System Settings ▸ Sound.

[`Driver`]: https://docs.rs/tympan-aspl
[`BundleConfig`]: https://docs.rs/tympan-aspl
